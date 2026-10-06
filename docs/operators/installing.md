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
  key pair that may read, write, delete, and list in it, and two things clients reach directly: the S3 API (they
  upload to presigned URLs, so it must allow your deployment's origin and the apps' by CORS) and an anonymous
  read path for downloads (a public bucket, a website endpoint, or a CDN in front of one). See
  [`[media.s3]`](configuration.md#medias3).

## 2. Building

```
cargo build --release -p aspen-chat-server -p aspen-migrate -p voice_server
```

The API server links the system's OpenSSL (for passkeys), so it needs OpenSSL's development
files. To show videos inline with a poster, it runs `ffmpeg` and `ffprobe`, which it finds on
`PATH`; without them it shows pictures inline and offers videos for download (see
[`[media.previews]`](configuration.md#mediapreviews)). The voice server builds mediasoup's C++ worker, which needs a C++ compiler, `make`, and
Python 3. The binaries land in `target/release/`.

For 64-bit ARM, such as a Raspberry Pi 5, `scripts/cross_aarch64.py` builds everything on an
x86-64 Linux machine; the binaries need only `libssl3` on the Pi.

Build the web client too. Every API server serves it, and will not start without it:

```
cd client
pnpm install
pnpm build
```

It lands in `client/packages/app/dist/`; copy that directory to each API server's machine.

## 3. Configuring and migrating

Write `aspen.toml` beside where the API server will run. The required settings are your
deployment's address, where the web client is, and the four services:

```toml
public_url = "https://chat.example.org"
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

[web_client]
dir = "/srv/aspen/dist"
```

`public_url` is the one address people use for everything: the API, the web client, and every
link the deployment hands out. Its host is also your deployment's
[federation](federation.md) domain and the domain passkeys belong to, so **choose it for good**:
once a server has started with an `https` address, a server started with another host refuses to
start, and a new host would make every passkey useless.

To send mail (email verification, password reset by email, the daily digest, and a newsletter),
add an SMTP server; without it the deployment works without email:

```toml
[email]
smtp_url = "smtps://aspen:…@smtp.example.org"
from = "Example Chat <noreply@chat.example.org>"
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

### Private workers

Every API server also does a share of the deployment's background work: sending mail and making
digests, applying the voice servers' reports, confirming visitors with their home deployments,
running plugins' observers and timers, and waking phones. To give that work machines of its own,
start a server as a private worker:

```
aspen-chat-server --private-worker
```

It opens no listening socket, so it takes no requests and holds no event streams, needs no web
client, and takes none of the listening or TLS flags. It reads the same `aspen.toml` and needs
the same services, and serves its metrics like any other when they are on. A private worker with
`[email] send` on, among servers with it off, is how the SMTP credentials stay on a machine that
takes no traffic.

## 5. Serving the web client

Each API server serves the web client itself, at `public_url` beside the API: its files from
[`[web_client] dir`](configuration.md#web_client), and every other path the API does not own
with its page, since the web client's links are real paths such as `/communities/…`. A reverse
proxy in front sends everything to the API servers, the WebSocket at `/api/v1/events` included
(pass `Upgrade` and `Connection` through). With Caddy:

```
chat.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

Files under `/assets/` are named by their contents and sent to be cached for good; everything
else is revalidated on each load.

### Security headers

The server sends the web client with a Content Security Policy and the other headers that keep
it to itself (`nosniff`, no referrer, framing refused, and, when `public_url` is `https`,
HSTS for a year: once a browser has seen it, it reaches your deployment over HTTPS only). The
policy is built from your configuration: it allows your storage's
[`public_base_url`](configuration.md#medias3) for pictures and videos and its `public_endpoint`
(or `endpoint`) for uploads, so nothing needs adding by hand. Let your reverse proxy pass these
headers through rather than setting its own: a second policy is applied as well as the first,
and one that leaves out your storage breaks pictures and uploads.

### Link previews

Chat apps, social networks, and search engines preview a link from the page it opens, without
running the web client, so each page is sent with link preview tags (Open Graph): your
deployment's name and icon, or, for an invite link, its community's. A deployment without an
icon previews with the Aspen mark, `open-graph.png` in the web client.

An invite link's preview shows its community's name and icon to anyone who has the link,
signed in or not, as long as the invite works; a revoked or expired invite previews as the
deployment. Services that unfurl links keep their own copy of a preview for a while, so a
renamed or deleted community can still show in previews made before.

### Releasing a new web client

The server reads the web client's files as they are asked for, so a new release needs no
restart: copy the new build over the old on each API server, `index.html` last. Copy rather
than replace the directory: browsers that already have the web client open load parts of it
(the code highlighting for each language, the QR code reader) only when they need them, from the
release they started with. Remove old files from `assets/` once a release has been out for a few
days, by deleting what the newest build does not have.

## 6. Voice servers

Each voice server is its own machine (or container) with its own `voice_server.toml`. First
say how many people it can carry: on the machine that will run it,

```
voice_server estimate-capacity
```

measures this CPU and reads the machine's memory, network, and port range, and prints a
capacity. Register the server where an API server runs:

```
aspen-chat-server voice-servers add voice-1 --url https://voice-1.chat.example.org --capacity 120
```

It may be run again with the same arguments, so a deployment script can run it every time; the
dashboard registers servers too. Its id is then in the database
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
permission but the moderation ones if there is none: `moderateCommunities` (reading everything in
any community or DM, the record of files sent in calls, and taking things out; it includes
`removeContent`), `reviewReports` (the reports people make of messages, profiles, and
nicknames, and warning the people reported), `removeContent` (deleting a reported message,
clearing a reported nickname, resetting a reported profile, and deleting a banned account's
recent messages), `banUsers` (banning accounts from the whole deployment), and `messageAnyUser`
(messaging anyone, past blocks). `admin allow <permission>` lets the top role do each of them
too. From then on, administrators manage everything else from the
Administration Dashboard, starting with its Settings tab: the display name and icon the
sign-in screen welcomes people with, and the deployment's policies.

A deployment open to anyone may skip this. To make it invite-only before anyone has an account,
turn that on from the terminal, then make the first invite:

```
aspen-chat-server settings set --registration-invite-required true
aspen-chat-server invites create
```

The rest of the [deployment settings](configuration.md#deployment-settings) can be set the same
way.

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
4. Copy the new web client over the old on each API server (see
   [Releasing a new web client](#releasing-a-new-web-client)).

People's event streams reconnect by themselves when an API server restarts, and pick up exactly
where they left off.
