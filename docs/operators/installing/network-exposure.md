# Network exposure

## What people must reach

Only these need to be reachable by the people using the deployment:

| What | Where |
| --- | --- |
| The API servers (or the reverse proxy before them) | TCP 443 at `public_url` |
| Each voice server's signalling | TCP 443 at its registered `url`, through its TLS proxy |
| Each voice server's media | UDP and TCP `min_port` to `max_port` |
| Each voice server's file transfers | UDP `[transfer] port` (3478) |
| The storage's S3 API, for uploads | `[media.s3] public_endpoint` |
| The storage's read path, for downloads | `[media.s3] public_base_url` (see [The storage's read path](storage-read-path.md)) |

## What must stay private

Keep everything else on a private network, or on loopback where it runs beside what uses it, and
firewall it from the internet. **Each of these trusts whoever reaches it.**

| Service | Ports | What reaching it gives |
| --- | --- | --- |
| PostgreSQL | 5432 | — |
| NATS | 4222, and its monitoring (8222) and cluster (6222) ports if they are on | Every event, and signing anyone into a call. |
| Valkey | 6379 | The codes being mailed, and the rate limits. |
| The storage's administration and internal ports | A SeaweedFS master, volume, and filer: 9333, 8080, 8888. Garage's RPC and admin ports. | The internals write without credentials. |
| The voice servers' `listen_addr` | Behind their proxy | — |
| Both servers' metrics | 9464 and 9465 | — |
| The tokio console | 6669, where it is built in | — |

## Docker

**Docker publishes ports past the host firewall.** A port published as `-p 5432:5432` (or
`ports: ["5432:5432"]` in a compose file) is opened on every interface, by rules Docker puts ahead
of `ufw` and `firewalld`, whatever those say. Instead, do one of these:

- Publish services only on loopback or a private address (`127.0.0.1:5432:5432`, as
  `docker-compose.yaml` does).
- Leave them unpublished, on a Docker network the servers share.
- Filter in the `DOCKER-USER` chain.

## PostgreSQL

**PostgreSQL is reached in plaintext unless `database_url` asks for TLS.** A server in the middle
can then read every query and, unless the certificate is checked, ask for the password in the
clear.

When the database is on another machine:

1. Give PostgreSQL a certificate: `ssl = on` with `ssl_cert_file` and `ssl_key_file`.
2. Add `hostssl` lines in `pg_hba.conf`, so it refuses plaintext.
3. Check the certificate from every server, in `database_url`:
   `postgres://aspen:…@db.internal/aspen?sslmode=verify-full&sslrootcert=/etc/aspen/db-ca.pem`.

Variations:

- For a certificate from a public authority, use `sslrootcert=system`. A managed database's
  usually needs its provider's bundle as the file.
- `sslmode=verify-ca` checks the authority but not the name, for a database reached by an
  address its certificate does not name.
- Where PostgreSQL authenticates by certificate (`cert` in `pg_hba.conf`), add
  `&sslcert=…&sslkey=…`.
- The servers sign in with SCRAM bound to the TLS session whenever PostgreSQL offers it.
  `&channel_binding=require` refuses any other sign-in.

See [`database_url`](../configuration/services.md#database_url) for every parameter.

## Valkey

Valkey has no password by default.

1. Set one: `requirepass`, or an ACL user.
2. Give it in `valkey_url`: `redis://:password@valkey.internal:6379`, or
   `redis://user:password@…`.

**Without TLS the password and everything stored cross the network readable.** So the servers
refuse a password in a `redis://` address to another machine. For Valkey on another machine:

1. Give Valkey a certificate: `tls-port`, `tls-cert-file`, `tls-key-file`, and `port 0` to refuse
   plaintext.
2. Name it with `rediss://` in `valkey_url`.
3. For a certificate from your own authority, give `[valkey.tls] ca_file`.
4. For a Valkey that asks for a client certificate (`tls-auth-clients yes`), give
   `[valkey.tls] cert_file` and `key_file`.

## NATS

NATS must have a token or users. [Step 6: Voice servers](6-voice-servers.md#give-it-a-nats-user)
gives the users.

Voice servers reach NATS, and often run elsewhere. When a voice server reaches NATS across a
network you do not control, do one of these:

- Give NATS a certificate (`tls { cert_file: …, key_file: … }` in its configuration), from a
  public authority or one the voice server's machine trusts. Name it with `tls://` in every
  `nats_url`.
- Connect the machines over a VPN.

**Without either, the NATS password and every event cross the network readable**, and every
server warns at startup.

With a certificate:

- From your own authority, give `[nats.tls] ca_file`, in `aspen.toml` and each
  `voice_server.toml`.
- With `verify: true` in NATS's `tls` block, give each server `[nats.tls] cert_file` and
  `key_file` too.
- `[nats.tls]` also makes TLS required, whatever `nats_url` names.

## Object storage and SMTP

Reach them over `https://` and `smtps://` (or `?tls=required`), like any other service.

- For a certificate from your own authority, give `[media.s3.tls] ca_file` and
  `[email.tls] ca_file`.
- For an SMTP server that asks for a client certificate, give `[email.tls] cert_file` and
  `key_file`.

---

Next: [The storage's read path](storage-read-path.md)
