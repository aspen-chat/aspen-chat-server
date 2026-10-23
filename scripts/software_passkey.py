"""A passkey held in software, for scripts that check a deployment's passkey ceremonies.

It registers and signs as a platform authenticator would, with "none" attestation and an RS256
key (which `webauthn-rs` accepts beside ES256, and which needs only the standard library to
sign), so a check needs no browser and no package beyond Python's own. Only for checks: its key
is made with Python's `secrets` and its arithmetic is not constant-time.
"""

from __future__ import annotations

import base64
import hashlib
import json
import secrets


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def unb64url(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def cbor(value) -> bytes:
    """The CBOR of the integers, byte and text strings, lists, and maps WebAuthn uses."""
    def head(major: int, length: int) -> bytes:
        if length < 24:
            return bytes([major << 5 | length])
        for extra, size in ((24, 1), (25, 2), (26, 4), (27, 8)):
            if length < 1 << (8 * size):
                return bytes([major << 5 | extra]) + length.to_bytes(size, "big")
        raise ValueError("too long for CBOR")

    if isinstance(value, int):
        return head(0, value) if value >= 0 else head(1, -1 - value)
    if isinstance(value, bytes):
        return head(2, len(value)) + value
    if isinstance(value, str):
        encoded = value.encode()
        return head(3, len(encoded)) + encoded
    if isinstance(value, list):
        return head(4, len(value)) + b"".join(cbor(item) for item in value)
    if isinstance(value, dict):
        return head(5, len(value)) + b"".join(cbor(k) + cbor(v) for k, v in value.items())
    raise TypeError(f"no CBOR for {type(value).__name__}")


def probable_prime(bits: int) -> int:
    """A random prime of `bits` bits, by Miller-Rabin."""
    while True:
        candidate = secrets.randbits(bits) | (1 << (bits - 1)) | (1 << (bits - 2)) | 1
        if all(candidate % small for small in (3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37)) and _miller_rabin(candidate):
            return candidate


def _miller_rabin(n: int, rounds: int = 32) -> bool:
    d, r = n - 1, 0
    while d % 2 == 0:
        d //= 2
        r += 1
    for _ in range(rounds):
        x = pow(secrets.randbelow(n - 3) + 2, d, n)
        if x in (1, n - 1):
            continue
        for _ in range(r - 1):
            x = pow(x, 2, n)
            if x == n - 1:
                break
        else:
            return False
    return True


# DigestInfo of SHA-256, before the digest, as PKCS #1 v1.5 signatures carry it.
SHA256_DIGEST_INFO = bytes.fromhex("3031300d060960864801650304020105000420")
FLAG_USER_PRESENT = 0x01
FLAG_USER_VERIFIED = 0x04
FLAG_ATTESTED_DATA = 0x40


class SoftwarePasskey:
    """One discoverable credential of the account whose registration options it is given."""

    def __init__(self, rp_id: str, origin: str):
        self.rp_id = rp_id
        self.origin = origin
        self.credential_id = secrets.token_bytes(16)
        self.user_handle = b""
        self.count = 0
        while True:
            p, q = probable_prime(1024), probable_prime(1024)
            phi = (p - 1) * (q - 1)
            if p != q and phi % 65537:
                break
        self.n, self.e = p * q, 65537
        self.d = pow(self.e, -1, phi)

    @property
    def id(self) -> str:
        return b64url(self.credential_id)

    def _client_data(self, kind: str, options: dict, origin: str | None) -> bytes:
        return json.dumps({
            "type": kind,
            "challenge": options["publicKey"]["challenge"],
            "origin": origin or self.origin,
            "crossOrigin": False,
        }).encode()

    def _sign(self, message: bytes) -> bytes:
        size = (self.n.bit_length() + 7) // 8
        digest = SHA256_DIGEST_INFO + hashlib.sha256(message).digest()
        padded = b"\x00\x01" + b"\xff" * (size - len(digest) - 3) + b"\x00" + digest
        return pow(int.from_bytes(padded, "big"), self.d, self.n).to_bytes(size, "big")

    def register(self, options: dict) -> dict:
        """The credential `navigator.credentials.create` would return, as JSON."""
        self.user_handle = unb64url(options["publicKey"]["user"]["id"])
        key = cbor({1: 3, 3: -257, -1: self.n.to_bytes(256, "big"), -2: self.e.to_bytes(3, "big")})
        auth_data = (hashlib.sha256(self.rp_id.encode()).digest()
                     + bytes([FLAG_USER_PRESENT | FLAG_USER_VERIFIED | FLAG_ATTESTED_DATA])
                     + self.count.to_bytes(4, "big")
                     + bytes(16)
                     + len(self.credential_id).to_bytes(2, "big") + self.credential_id
                     + key)
        attestation = cbor({"fmt": "none", "attStmt": {}, "authData": auth_data})
        return {
            "id": self.id,
            "rawId": self.id,
            "type": "public-key",
            "clientExtensionResults": {},
            "response": {
                "clientDataJSON": b64url(self._client_data("webauthn.create", options, None)),
                "attestationObject": b64url(attestation),
                "transports": ["internal"],
            },
        }

    def assert_(self, options: dict, origin: str | None = None) -> dict:
        """The credential `navigator.credentials.get` would return, as JSON, signed as made
        at `origin` (the page's own by default; an Android app's `android:apk-key-hash:`)."""
        self.count += 1
        auth_data = (hashlib.sha256(self.rp_id.encode()).digest()
                     + bytes([FLAG_USER_PRESENT | FLAG_USER_VERIFIED])
                     + self.count.to_bytes(4, "big"))
        client_data = self._client_data("webauthn.get", options, origin)
        signature = self._sign(auth_data + hashlib.sha256(client_data).digest())
        return {
            "id": self.id,
            "rawId": self.id,
            "type": "public-key",
            "clientExtensionResults": {},
            "response": {
                "clientDataJSON": b64url(client_data),
                "authenticatorData": b64url(auth_data),
                "signature": b64url(signature),
                "userHandle": b64url(self.user_handle),
            },
        }
