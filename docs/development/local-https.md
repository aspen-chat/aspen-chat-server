# HTTPS in development

A browser treats plain `http://localhost` as secure, so a development deployment used only from the
machine it runs on needs no certificate: the Vite dev server at `http://localhost:5173`, proxying to
`cargo run -- --no-https --port 8000`, has passkeys, Web Crypto, and service workers. Nothing else
is plain HTTP. A server refuses an `http` `public_url` anywhere but this machine, refuses to start
without a certificate unless told `--no-https`, and an `https` deployment refuses `http` voice
servers. So to reach a development deployment from a phone, another computer, or a simulator, or to
work on anything TLS changes, serve it over HTTPS with a certificate from the development
certificate authority.

## The development authority

```
scripts/dev_ca.py issue aspen.localhost          # prints the certificate, its key, and ca.pem
scripts/dev_ca.py path                           # the authority's certificate
```

The authority is made once, in `target/dev-ca/`, and every certificate is issued from it there.
`scripts/dev_federation.py` and the push stand-ins issue theirs from it too, so trusting it once
covers every development deployment.

It may sign only for development names, which every client enforces:

- `localhost` and names under it,
- names under `home.arpa`, `internal`, `test`, `nip.io`, and `sslip.io`,
- loopback addresses and private ones (`10/8`, `172.16/12`, `192.168/16`, `fc00::/7`).

Its key never signs for any other site, so a device that trusts it trusts nothing more. Still,
**`target/dev-ca/ca.key` is the authority**: never copy the directory anywhere, only `ca.pem`.

## Trusting it

Install `target/dev-ca/ca.pem` wherever a client should reach a development deployment.

| Where                          | How                                                                                                                                                                                                                                    |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Arch, Fedora                   | `sudo trust anchor --store target/dev-ca/ca.pem`                                                                                                                                                                                       |
| Debian, Ubuntu                 | `sudo cp target/dev-ca/ca.pem /usr/local/share/ca-certificates/aspen-dev-ca.crt && sudo update-ca-certificates`                                                                                                                        |
| Chrome, Chromium, Electron     | On Linux they read the NSS database: `certutil -d sql:$HOME/.pki/nssdb -A -t C,, -n "Aspen development" -i target/dev-ca/ca.pem`. On macOS and Windows, the system's store.                                                            |
| Firefox                        | Its own store: Settings, Privacy & Security, Certificates, View Certificates, Authorities, Import, and tick "Trust this CA to identify websites".                                                                                      |
| macOS                          | `sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ca.pem`                                                                                                                                          |
| Windows                        | `certutil -addstore -f Root ca.pem` from an administrator's prompt, or `certutil -user -addstore Root ca.pem` for one account.                                                                                                         |
| iOS                            | Open `ca.pem` on the device (AirDrop, or mail it), install the profile under Settings, General, VPN & Device Management, then turn it on under Settings, General, About, Certificate Trust Settings. The app and Safari both trust it. |
| iOS Simulator                  | `xcrun simctl keychain booted add-root-cert ca.pem`, or the UI tests' `--trust` (`client/docs/architecture/ios-app.md`).                                                                                                               |
| Android, and its emulator      | Settings, Security, Encryption & credentials, Install a certificate, CA certificate. Chrome and Aspen's debug build trust it; a release build of Aspen trusts only the system's authorities.                                           |
| Node (Playwright, the scripts) | `NODE_EXTRA_CA_CERTS=target/dev-ca/ca.pem`. The Vite dev server's proxy already checks an `https` target against it.                                                                                                                   |
| Python                         | `ssl.create_default_context(cafile="target/dev-ca/ca.pem")`, as `scripts/dev_federation.py` does.                                                                                                                                      |
| Another deployment             | `[federation.development] extra_root_certificates`, for deployments side by side at `localhost` (`scripts/dev_federation.py`).                                                                                                         |

## On this machine alone

A name under `localhost` needs no DNS:

```
scripts/dev_ca.py issue aspen.localhost
cargo run -- --port 8443 --cert target/dev-ca/aspen.localhost.pem --key target/dev-ca/aspen.localhost.key
```

with `public_url = "https://aspen.localhost:8443"`. The Vite dev server serves HTTPS too when given
a certificate, in `client/packages/app/.env.local` or the environment:

```
VITE_DEV_HTTPS_CERT=../../../target/dev-ca/aspen.localhost.pem
VITE_DEV_HTTPS_KEY=../../../target/dev-ca/aspen.localhost.key
VITE_DEV_PROXY_TARGET=https://aspen.localhost:8443
```

## From other devices

Other devices need a name that resolves to this machine. `nip.io` gives every address one with no
setup: `192-168-2-220.nip.io` resolves to `192.168.2.220`, and so does every name under it. (A name
under `home.arpa` in your router's DNS works the same, without depending on `nip.io`.) Caddy, the
`caddy` service in `docker-compose.yaml` (`scripts/caddy/Caddyfile`), terminates TLS for the API
server, the voice server, and SeaweedFS, each at a name of its own on port 443:

| Address               | Proxied to                         |
| --------------------- | ---------------------------------- |
| `https://$HOST`       | the API server, `127.0.0.1:8000`   |
| `https://voice.$HOST` | the voice server, `127.0.0.1:9001` |
| `https://s3.$HOST`    | SeaweedFS's S3 API, uploads        |
| `https://media.$HOST` | SeaweedFS's filer, downloads       |

1. Issue a certificate for the host and the names under it, and start Caddy:

   ```
   HOST=192-168-2-220.nip.io
   scripts/dev_ca.py issue $HOST "*.$HOST"
   ASPEN_DEV_HOST=$HOST docker compose --profile https up -d caddy
   ```

   Caddy is what other devices reach; leave `ASPEN_DEV_BIND` unset, so the database, NATS, Valkey,
   and storage listen on this machine alone.

2. Give the deployment credentials of its own. A server whose `public_url` is not at this machine
   counts as one others may reach, and refuses the passwords and keys this repository publishes
   (`docs/operators/configuration/services.md`):
   - the database: a role of its own,
     `CREATE ROLE aspen_dev LOGIN SUPERUSER PASSWORD '<long random string>'`, named in
     `database_url`;
   - NATS: `ASPEN_DEV_NATS_TOKEN=<long random string>` in `.env` beside `docker-compose.yaml`, then
     `docker compose up -d nats`, and the same token as `nats_auth_token` in `aspen.toml` and
     `voice_server.toml`;
   - storage: an S3 identity of its own, added beside the repository's in SeaweedFS's shell
     (`weed shell`, then
     `s3.configure -user=<name> -access_key=<…> -secret_key=<…> -actions=Read,Write,List,Tagging -buckets=aspen-media -apply`),
     named in `[media.s3]`.

3. Serve the API server behind Caddy, and name the proxied addresses:

   ```toml
   public_url = "https://192-168-2-220.nip.io"

   [rate_limits]
   trusted_proxies = ["127.0.0.1"]

   [media.s3]
   endpoint = "http://127.0.0.1:8333"
   public_endpoint = "https://s3.192-168-2-220.nip.io"
   public_base_url = "https://media.192-168-2-220.nip.io/buckets/aspen-media"
   ```

   ```
   cargo run -- --no-https --listen-addr 127.0.0.1 --port 8000
   aspen-chat-server voice-servers set <name> --url https://voice.192-168-2-220.nip.io
   ```

4. Trust `ca.pem` on each device, and open `https://192-168-2-220.nip.io`.

The voice server's media is UDP straight to the machine, as on any deployment; only its signalling
goes through Caddy.
