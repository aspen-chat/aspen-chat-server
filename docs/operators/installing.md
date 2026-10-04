# Installing Aspen

## 1. The services

Aspen needs four services. `docker-compose.yaml` in the repository starts all four for
development, with passwords written into it; it is not a production setup.

- **PostgreSQL**, any supported version. The migrations create the `pg_trgm` extension, which
  the database's owner may do on PostgreSQL 13 and later.
- **NATS 2.10 or later, with JetStream on** (`--jetstream`) and a token (`--auth <token>`).
  Aspen creates the stream it needs and keeps only the last minute of events, in memory.
- **Valkey** (or anything that speaks the Redis protocol).
- **Object storage that speaks S3**: SeaweedFS, Garage, MinIO, or AWS S3. It needs a bucket, a
  key pair that may read and write it, and two things clients reach directly: the S3 API (they
  upload to presigned URLs, so it must allow your web client's origin by CORS) and an anonymous
  read path for downloads (a public bucket, a website endpoint, or a CDN in front of one). See
  [`[media.s3]`](configuration.md#medias3).

## 2. Building

```
cargo build --release -p aspen-chat-server -p aspen-migrate -p voice_server
```

The API server links the system's OpenSSL (for passkeys), so it needs OpenSSL's development
files. The voice server builds mediasoup's C++ worker, which needs a C++ compiler, `make`, and
Python 3. The binaries land in `target/release/`.

For 64-bit ARM, such as a Raspberry Pi 5, `scripts/cross_aarch64.py` builds everything on an
x86-64 Linux machine; the binaries need only `libssl3` on the Pi.

The web client is built separately:

```
cd client
pnpm install
pnpm build
```

It lands in `client/packages/app/dist/`, a static site.

## 3. Configuring and migrating

Write `aspen.toml` beside where the API server will run. The required settings are the four
services:

```toml
database_url = "postgres://aspen:…@db.internal/aspen"
nats_url = "nats.internal:4222"
nats_auth_token = "…"
valkey_url = "redis://valkey.internal:6379"

[media.s3]
endpoint = "http://storage.internal:8333"
public_endpoint = "https://storage.chat.example.org"
public_base_url = "https://media.chat.example.org/aspen-media"
bucket = "aspen-media"
region = "us-east-1"
access_key = "…"
secret_key = "…"

[voice]
token_secret = "a long random string, shared with every voice server"
```

Every other setting has a default; [Configuration](configuration.md) lists them all. Each can
also be given in the environment, which overrides the file: `ASPEN_` and the key, with `__`
between nested keys (`ASPEN_DATABASE_URL`, `ASPEN_VOICE__TOKEN_SECRET`).

Then create the tables:

```
aspen-migrate up
```

It reads the database from `--database-url`, then `DATABASE_URL`, then `database_url` in
`aspen.toml`. Run it again after every upgrade, before starting the new servers; it applies
only what has not been applied.

## 4. Starting the API server

With a certificate for your domain:

```
aspen-chat-server --cert fullchain.pem --key privkey.pem
```

It listens on port 443 of every interface (`--port`, `--listen-addr`, which may be given more
than once). It does not notice a renewed certificate: restart it after renewing.

Without `--cert` and `--key` it makes a self-signed certificate for `localhost`, which only
suits development.

Behind a reverse proxy that terminates TLS, run it without TLS on a private address:

```
aspen-chat-server --no-https --listen-addr 127.0.0.1 --port 8080
```

and tell it which proxies to believe about client addresses, or every client will be counted as
the proxy by the rate limits:

```toml
[rate_limits]
trusted_proxies = ["127.0.0.1"]
```

Run as many API servers as you need; they share everything through the services, and any of
them can serve any request.

## 5. Serving the web client

Serve the web client and the API from the same origin, `https://chat.example.org`: the web client
then talks to the server it was loaded from, and needs no CORS. Route these to the API server:

- `/api/` (including the WebSocket at `/api/v1/events`: pass `Upgrade` and `Connection` through)
- `/auth/passkey` (the page the apps open to use a passkey)
- `/.well-known/aspen` (the federation document)

and everything else to `client/packages/app/dist/`, answering any path that is not a file with
`index.html`, since the web client's links are real paths such as `/communities/…`.

With Caddy, that is:

```
chat.example.org {
    @api path /api/* /auth/passkey /.well-known/aspen
    reverse_proxy @api 127.0.0.1:8080
    root * /srv/aspen/dist
    try_files {path} /index.html
    file_server
}
```

Serving the web client from another origin works too: list that origin in
[`[cors] allowed_origins`](configuration.md#cors), and build the client with
`VITE_ASPEN_SERVER_URL=https://api.example.org` so it knows where the API is.

## 6. Voice servers

Each voice server is its own machine (or container) with its own `voice_server.toml`. First
say how many people it can carry: on the machine that will run it,

```
voice_server estimate-capacity
```

measures this CPU and reads the machine's memory, network, and port range, and prints a
capacity. Register the server with the API servers by adding it to `aspen.toml`:

```toml
[[voice.servers]]
name = "voice-1"
url = "https://voice-1.chat.example.org"
capacity = 120
```

and restart them; they record it at startup. Its id is then in the database
(`SELECT id FROM voice_server WHERE name = 'voice-1'`). Give the voice server that id, the same
`token_secret`, and NATS:

```toml
id = "…"
token_secret = "the same string as [voice] token_secret"
nats_url = "nats.internal:4222"
nats_auth_token = "…"
listen_addr = "127.0.0.1:9000"

[rtc]
announced_address = "203.0.113.10"
min_port = 40000
max_port = 40999
```

Clients reach a voice server in two ways, and both must be open to them:

- **Signalling and the latency check**, over HTTPS: `GET /health` and the WebSocket
  `GET /ws`. The voice server speaks plain HTTP on `listen_addr`, so put a TLS proxy in front
  of it at the `url` you registered, passing WebSocket upgrades through.
- **Media**, over UDP (and TCP where UDP is blocked) on the ports from `min_port` to
  `max_port`. `announced_address` is the address clients send media to: set it to the
  server's public address when it is behind NAT. Leave it out only when the machine has a
  public address on an interface. Never set it to a loopback address.
- **File transfers**, over UDP port `[transfer] port` (3478): STUN, so that two people's
  devices can connect directly, and the TURN relay for transfers that go through the server,
  at no more than `[transfer] relay_mbps` (50) in all. Set `relay_mbps = 0` not to relay
  transfers at all.

A voice server that stops reporting for a minute is no longer offered to people joining calls;
one that people fail to reach is disabled after `failure_threshold` of them try in
`failure_window_seconds`, until an administrator enables it again in the dashboard.

## 7. The first administrator

Create an account from the web client, then, where the API server runs:

```
aspen-chat-server admin grant <username>
```

It needs the database and NATS, as the servers do: it announces the change to the account's open
apps. It gives that account the deployment's top role, making an Administrator role with every
permission but the moderation ones if there is none: `moderateCommunities` (reading and taking
things out of any community or DM), `reviewReports` (the reports people make of messages and
profiles), `banUsers` (banning accounts from the whole deployment), and `messageAnyUser`
(messaging anyone, which a moderator's warning takes). `admin allow <permission>` lets the top
role do each of them too. From then on, administrators manage everything else from the
Administration Dashboard, starting with its Profile tab: the display name and icon the
sign-in screen welcomes people with.

If `[registration] invite_required` is on, nobody can create the first account without an
invite: make one first with `aspen-chat-server invites create`.

Commands like these read the same `aspen.toml`, so run them from the same directory, with the
same environment, as the server.

## 8. Checking it works

`scripts/smoke_servers.py --bin target/release` starts a throwaway database, both servers,
registers a user, searches, joins a call through the voice server, and reads both metrics
endpoints, then cleans up. It needs the services from `docker-compose.yaml`.

Both servers export Prometheus metrics on loopback (`127.0.0.1:9464` and `127.0.0.1:9465`);
scrape them from the same machine, and keep them off public interfaces.

## Upgrading

1. Build the new binaries and web client.
2. Run `aspen-migrate up`.
3. Restart the API servers, then the voice servers. Calls on a voice server that restarts end,
   and their clients rejoin on their own.
4. Replace the web client's files.

People's event streams reconnect by themselves when an API server restarts, and pick up exactly
where they left off.
