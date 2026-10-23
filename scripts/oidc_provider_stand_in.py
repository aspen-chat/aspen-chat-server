#!/usr/bin/env python3
"""A stand-in OpenID Connect provider, for checking sign-in through identity providers
(`docs/architecture/sign-in-and-security/identity-providers.md`) without a real one.

    from oidc_provider_stand_in import Provider
    with Provider(port=18443) as provider:
        provider.sign_in_next(sub="alice", amr=["pwd"], sid="s-1")
        ...
        token = provider.logout_token(sid="s-1")

It serves discovery, keys, authorization, and tokens over HTTPS with a certificate for
`localhost` from the development authority (`dev_ca.py`), so a server trusts it with
`[identity_providers] ca_file` naming that authority and `allow_private_addresses` on. It shows
no sign-in page: `/authorize` sends the browser straight back with a code for whichever identity
`sign_in_next` set up, or with an error. It signs ID tokens and logout tokens with RS256 through
the `openssl` command, so it needs nothing beyond Python's standard library and that, as
`dev_ca.py` does. It checks PKCE, the client's secret, and the redirect URI as a provider would.

Run on its own (`scripts/oidc_provider_stand_in.py --port 18443`), it signs in everyone as the
identity its flags give, for trying the web client's sign-in by hand.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.server
import json
import secrets
import ssl
import subprocess
import tempfile
import threading
import time
import urllib.parse
from pathlib import Path

import dev_ca

CLIENT_ID = "aspen-stand-in"
CLIENT_SECRET = "stand-in-secret"
LOGOUT_EVENT = "http://schemas.openid.net/event/backchannel-logout"


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


class Signer:
    """An RSA key made with `openssl`, signing RS256 JSON Web Tokens."""

    def __init__(self, work: Path):
        self.key = work / "signing.key"
        subprocess.run(["openssl", "genrsa", "-out", str(self.key), "2048"], check=True, capture_output=True)
        modulus = subprocess.run(["openssl", "rsa", "-in", str(self.key), "-noout", "-modulus"],
                                 check=True, capture_output=True, text=True).stdout.strip().split("=", 1)[1]
        self.kid = secrets.token_hex(8)
        self.jwk = {"kty": "RSA", "use": "sig", "alg": "RS256", "kid": self.kid,
                    "n": b64url(bytes.fromhex(modulus)), "e": b64url((65537).to_bytes(3, "big"))}

    def sign(self, claims: dict, typ: str = "JWT") -> str:
        header = b64url(json.dumps({"alg": "RS256", "kid": self.kid, "typ": typ}).encode())
        payload = b64url(json.dumps(claims).encode())
        signing_input = f"{header}.{payload}".encode()
        signature = subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(self.key)], input=signing_input,
                                   check=True, capture_output=True).stdout
        return f"{header}.{payload}.{b64url(signature)}"


class Provider:
    """The stand-in, served on `port` until the block ends."""

    def __init__(self, port: int, client_id: str = CLIENT_ID, client_secret: str = CLIENT_SECRET):
        self.port = port
        self.client_id = client_id
        self.client_secret = client_secret
        self.issuer = f"https://localhost:{port}"
        self.work = Path(tempfile.mkdtemp(prefix="aspen-oidc-stand-in-"))
        self.signer = Signer(self.work)
        self.next: dict = {"sub": "stand-in-user"}
        self.codes: dict[str, dict] = {}
        self.lock = threading.Lock()
        self.server: http.server.ThreadingHTTPServer | None = None

    def __enter__(self) -> "Provider":
        cert, key = dev_ca.issue("localhost")
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(cert), str(key))
        provider = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args) -> None:
                pass

            def do_GET(self) -> None:
                provider.handle_get(self)

            def do_POST(self) -> None:
                provider.handle_post(self)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", self.port), Handler)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        return self

    def __exit__(self, *_) -> None:
        if self.server is not None:
            self.server.shutdown()
            self.server.server_close()

    @staticmethod
    def authority() -> Path:
        """The certificate authority a server must trust for the stand-in."""
        return dev_ca.authority()

    def sign_in_next(self, sub: str, amr: list[str] | None = None, sid: str | None = None,
                     email: str | None = None, email_verified: bool = True, preferred_username: str | None = None,
                     name: str | None = None, error: str | None = None) -> None:
        """Whom the next authorization signs in, or the OAuth `error` it answers with instead."""
        with self.lock:
            self.next = {"sub": sub, "amr": amr, "sid": sid, "email": email, "email_verified": email_verified,
                         "preferred_username": preferred_username, "name": name, "error": error}

    def logout_token(self, sub: str | None = None, sid: str | None = None, **changes) -> str:
        """A back-channel logout token naming `sid` or `sub`, with `changes` laid over its claims."""
        claims = {"iss": self.issuer, "aud": self.client_id, "iat": int(time.time()), "jti": secrets.token_hex(12),
                  "events": {LOGOUT_EVENT: {}}}
        if sub is not None:
            claims["sub"] = sub
        if sid is not None:
            claims["sid"] = sid
        claims.update(changes)
        return self.signer.sign(claims, typ="logout+jwt")

    def document(self) -> dict:
        return {
            "issuer": self.issuer,
            "authorization_endpoint": f"{self.issuer}/authorize",
            "token_endpoint": f"{self.issuer}/token",
            "jwks_uri": f"{self.issuer}/jwks",
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
            "code_challenge_methods_supported": ["S256"],
            "backchannel_logout_supported": True,
            "backchannel_logout_session_supported": True,
        }

    @staticmethod
    def send(handler, status: int, body: dict | None = None, headers: dict | None = None) -> None:
        handler.send_response(status)
        for key, value in (headers or {}).items():
            handler.send_header(key, value)
        data = json.dumps(body).encode() if body is not None else b""
        if body is not None:
            handler.send_header("content-type", "application/json")
        handler.send_header("content-length", str(len(data)))
        handler.end_headers()
        handler.wfile.write(data)

    def handle_get(self, handler) -> None:
        url = urllib.parse.urlsplit(handler.path)
        if url.path == "/.well-known/openid-configuration":
            return self.send(handler, 200, self.document())
        if url.path == "/jwks":
            return self.send(handler, 200, {"keys": [self.signer.jwk]})
        if url.path != "/authorize":
            return self.send(handler, 404, {"error": "not_found"})
        query = dict(urllib.parse.parse_qsl(url.query))
        redirect = query.get("redirect_uri", "")
        if query.get("client_id") != self.client_id or not redirect:
            return self.send(handler, 400, {"error": "invalid_request"})
        with self.lock:
            identity = dict(self.next)
        back = {"state": query.get("state", "")}
        if identity.get("error"):
            back["error"] = identity["error"]
        else:
            code = secrets.token_urlsafe(24)
            with self.lock:
                self.codes[code] = {"identity": identity, "nonce": query.get("nonce"), "redirect": redirect,
                                    "challenge": query.get("code_challenge"), "auth_time": int(time.time())}
            back["code"] = code
        self.send(handler, 302, headers={"location": f"{redirect}?{urllib.parse.urlencode(back)}"})

    def handle_post(self, handler) -> None:
        if urllib.parse.urlsplit(handler.path).path != "/token":
            return self.send(handler, 404, {"error": "not_found"})
        length = int(handler.headers.get("content-length", "0"))
        form = dict(urllib.parse.parse_qsl(handler.rfile.read(length).decode()))
        client_id, client_secret = form.get("client_id"), form.get("client_secret")
        authorization = handler.headers.get("authorization", "")
        if authorization.lower().startswith("basic "):
            pair = base64.b64decode(authorization[6:]).decode()
            client_id, client_secret = (urllib.parse.unquote_plus(p) for p in pair.split(":", 1))
        if client_id != self.client_id or client_secret != self.client_secret:
            return self.send(handler, 401, {"error": "invalid_client"})
        with self.lock:
            grant = self.codes.pop(form.get("code", ""), None)
        if grant is None or form.get("redirect_uri") != grant["redirect"]:
            return self.send(handler, 400, {"error": "invalid_grant"})
        verifier = form.get("code_verifier", "")
        if b64url(hashlib.sha256(verifier.encode()).digest()) != grant["challenge"]:
            return self.send(handler, 400, {"error": "invalid_grant", "error_description": "PKCE"})
        identity = grant["identity"]
        now = int(time.time())
        claims = {"iss": self.issuer, "aud": self.client_id, "sub": identity["sub"], "iat": now, "exp": now + 300,
                  "auth_time": grant["auth_time"], "nonce": grant["nonce"]}
        for claim in ["amr", "sid", "email", "preferred_username", "name"]:
            if identity.get(claim) is not None:
                claims[claim] = identity[claim]
        if identity.get("email") is not None:
            claims["email_verified"] = identity["email_verified"]
        self.send(handler, 200, {"access_token": secrets.token_urlsafe(24), "token_type": "Bearer",
                                 "expires_in": 300, "id_token": self.signer.sign(claims)},
                  headers={"cache-control": "no-store"})


def main() -> None:
    parser = argparse.ArgumentParser(description="A stand-in OpenID Connect provider for development.")
    parser.add_argument("--port", type=int, default=18443)
    parser.add_argument("--sub", default="stand-in-user")
    parser.add_argument("--username", default="standin")
    parser.add_argument("--email", default=None)
    parser.add_argument("--amr", default="pwd", help="comma separated")
    args = parser.parse_args()
    with Provider(args.port) as provider:
        provider.sign_in_next(sub=args.sub, amr=args.amr.split(","), sid=secrets.token_hex(6),
                              email=args.email, preferred_username=args.username, name=args.username)
        print(f"issuer {provider.issuer}\nclient id {provider.client_id}\nclient secret {provider.client_secret}\n"
              f"trust {provider.authority()} with [identity_providers] ca_file, and set allow_private_addresses",
              flush=True)
        try:
            threading.Event().wait()
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
