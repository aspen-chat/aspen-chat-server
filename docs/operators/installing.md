# Installing Aspen

## 1. The services

Aspen needs four services. `docker-compose.yaml` in the repository starts all four for
development, with passwords written into it; it is not a production setup.

- **PostgreSQL**, any supported version. The migrations create the `pg_trgm` extension, which
  the database's owner may do on PostgreSQL 13 and later.
- **NATS 2.10 or later, with JetStream on** (`--jetstream`) and a token (`--auth <token>`).
  Aspen creates the stream it needs and keeps only the last minute of events, in memory.
- **Valkey** (or anything that speaks the Redis protocol). Give it a memory limit and tell it
  never to evict, `maxmemory 512mb` and `maxmemory-policy noeviction` in `valkey.conf` (or
  `--maxmemory 512mb --maxmemory-policy noeviction`). Everything Aspen keeps there expires on its
  own; an eviction policy would instead drop rate limit counts (letting guesses through) or
  sign-ins in progress at random, while a full Valkey that refuses writes is treated as one that
  cannot be reached: sign-in, password reset, registration, and invite endpoints answer
  `serverBusy` until it has room. Aspen limits how fast strangers can start what it stores there
  (password resets, passkey ceremonies, device links) for everyone together, so half a gigabyte
  is plenty for most deployments; watch `used_memory` in `INFO memory` and raise it if it
  approaches the limit.
- **Object storage that speaks S3**: SeaweedFS, Garage, MinIO, or AWS S3. It needs a bucket, a
  key pair that may read, write, delete, and list in it, and two things clients reach directly: the S3 API (they
  upload to presigned URLs, so it must allow your deployment's origin and the apps' by CORS) and an anonymous
  read path for downloads (a website endpoint, or a CDN in front of the bucket) that allows
  anonymous reads of objects and nothing else. See [`[media.s3]`](configuration.md#medias3) and
  [The storage's read path](#the-storages-read-path).

None of the four belongs on the internet: see [Network exposure](#network-exposure).

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

What the server fetches from addresses others choose (link previews, plugins' calls, other
deployments, and the push services phones name) it fetches directly, refusing private
addresses, and ignores `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`: a proxy would resolve names
itself, past that check. Let the API servers reach the internet on port 443 (and 80, for link
previews) without one.

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
it to itself (`nosniff`, no referrer, framing refused, no window shared with another page, and,
when `public_url` is `https`, HSTS for a year: once a browser has seen it, it reaches your
deployment over HTTPS only), and every API answer with `nosniff` and, but for the few meant to
be kept, `Cache-Control: no-store`, so no cache between keeps one user's answers. The
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
listen_addr = "127.0.0.1:9000"

[nats]
user = "voice-1"
password = "…"

[rtc]
announced_address = "203.0.113.10"
min_port = 40000
max_port = 40999
```

A voice server runs on a machine people send media to, so give it a NATS user of its own that
may do no more than a voice server does, rather than the token the API servers use, which lets
whoever holds it read and write every event of the deployment. NATS then signs everyone in as a
user, so the API servers get one too: replace their `nats_auth_token` with

```toml
[nats]
user = "aspen"
password = "…"
```

and start NATS with a configuration naming both (one entry like `voice-1` for each voice server,
with its own id in place of `VOICE_SERVER_ID`):

```
jetstream {}
authorization {
  users = [
    { user: "aspen", password: "…" }
    { user: "voice-1", password: "…", permissions: {
        publish: { allow: [
          "aspen.voice.report.*.VOICE_SERVER_ID",
          "aspen.voice.speaking.*.VOICE_SERVER_ID",
          "$JS.API.INFO",
          "$JS.API.STREAM.INFO.KV_aspen_rate_limits",
          "$JS.API.CONSUMER.CREATE.KV_aspen_rate_limits",
          "$JS.API.CONSUMER.CREATE.KV_aspen_rate_limits.>",
          "$JS.API.CONSUMER.DELETE.KV_aspen_rate_limits.>",
          "$JS.API.DIRECT.GET.KV_aspen_rate_limits.>",
          "$JS.API.STREAM.MSG.GET.KV_aspen_rate_limits",
          "$JS.FC.KV_aspen_rate_limits.>"
        ] }
        subscribe: { allow: [
          "aspen.voice.command.VOICE_SERVER_ID",
          "_INBOX_voice.VOICE_SERVER_ID.>"
        ] }
    } }
  ]
}
```

That lets the voice server publish its own reports (`aspen.voice.report.{lane}.{id}` and
`aspen.voice.speaking.{lane}.{id}`), receive its own commands, follow a suspension of rate limits
(the key-value bucket `aspen_rate_limits`), and receive the replies to its own requests, which it
asks for under `_INBOX_voice.{id}` rather than NATS's shared `_INBOX`. The API servers apply a
report only when it came on a subject naming the server it is about, and only when the call or
channel it is about is that server's, so a voice server taken over can misreport its own calls and
no one else's. A voice server still given `nats_auth_token` works, and warns at startup.

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
one that people fail to reach is suspended for `failure_window_seconds` after `failure_threshold`
of them try in that time, unless it is the last one taking calls.

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

## Network exposure

Only these need to be reachable by the people using the deployment:

| What | Where |
| --- | --- |
| The API servers (or the reverse proxy before them) | TCP 443 at `public_url` |
| Each voice server's signalling | TCP 443 at its registered `url`, through its TLS proxy |
| Each voice server's media | UDP and TCP `min_port` to `max_port` |
| Each voice server's file transfers | UDP `[transfer] port` (3478) |
| The storage's S3 API, for uploads | `[media.s3] public_endpoint` |
| The storage's read path, for downloads | `[media.s3] public_base_url` |

Everything else stays on a private network, or on loopback where it runs beside what uses it,
and is firewalled from the internet: PostgreSQL (5432), NATS (4222, and its monitoring 8222 and
cluster 6222 ports if they are on), Valkey (6379), the storage's administration and internal
ports (a SeaweedFS master, volume, and filer: 9333, 8080, 8888; Garage's RPC and admin ports),
the voice servers' `listen_addr` (behind their proxy), both servers' metrics (9464 and 9465),
and the tokio console (6669) where it is built in. Each of them trusts whoever reaches it: NATS
carries every event and can sign anyone into a call, Valkey holds the codes being mailed and the
rate limits, and the storage's internals write without credentials.

**Docker publishes ports past the host firewall.** A port published as `-p 5432:5432` (or
`ports: ["5432:5432"]` in a compose file) is opened on every interface by rules Docker puts ahead
of `ufw` and `firewalld`, whatever those say. Publish services only on loopback or a private
address (`127.0.0.1:5432:5432`, as `docker-compose.yaml` does), leave them unpublished on a
Docker network the servers share, or filter in the `DOCKER-USER` chain.

**Valkey** has no password by default. Set one (`requirepass`, or an ACL user) and give it in
`valkey_url` (`redis://:password@valkey.internal:6379`, or `redis://user:password@…`). The API
servers speak to Valkey without TLS, so keep it on the same machine or a private network; across
anything else, carry it over a VPN such as WireGuard.

**NATS** must have a token or users ([Voice servers](#6-voice-servers) gives the users), and is
reached by voice servers, which often run elsewhere. When a voice server reaches NATS across a
network you do not control, give NATS a certificate (`tls { cert_file: …, key_file: … }` in its
configuration; one from a public authority, or one the voice server's machine trusts) and name it
with `tls://` in every `nats_url`, or connect the machines over a VPN. Without either, the NATS
password and every event cross the network readable.

### The storage's read path

`public_base_url` is fetched by everyone's apps without credentials, so it must allow exactly
one thing: reading an object by its name (S3's `GetObject`). It must not list the bucket, which
would hand anyone every attachment ever posted, nor take writes or deletions.

- **AWS S3** (or anything taking its policies): a bucket policy allowing `s3:GetObject` on
  `arn:aws:s3:::BUCKET/*` to `*`, and nothing else; not `s3:ListBucket`.
- **MinIO**: `mc anonymous set download` also grants listing. Set a policy of your own with
  `mc anonymous set-json`, holding only the `s3:GetObject` statement above.
- **Garage**: its website endpoint (`s3_web`, `garage bucket website --allow BUCKET`) serves
  objects and does not list them.
- **SeaweedFS**: give the S3 gateway an anonymous identity allowed only `Read` on the bucket
  (`"actions": ["Read:BUCKET"]` in its S3 configuration) and serve `public_base_url` from the S3
  gateway or a CDN before it. **Never expose the filer** (port 8888): it lists directories and
  takes uploads and deletions from anyone.

Serve `public_base_url` from an origin of its own (`https://media.chat.example.org`), never under
`public_url`'s, so nothing posted can act as your deployment's pages, and have it (or the CDN
before it) send `X-Content-Type-Options: nosniff`, so a browser opens a file only as the type it
was stored as.

Check it from a machine outside your network, with the address of any picture someone posted
(copy it from the app) as `OBJECT`, `public_base_url` as `BASE`, and `public_endpoint` with the
bucket as `S3` (`https://s3.chat.example.org/aspen-media`):

```
curl -s -o /dev/null -w '%{http_code}\n' "$OBJECT"                     # 200
curl -sI "$OBJECT" | grep -i x-content-type-options                    # nosniff
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/"                      # 403 or 404, never 200
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/?list-type=2"          # 403 or 404, never 200
curl -s -o /dev/null -w '%{http_code}\n' "$S3?list-type=2"             # 403
curl -s -o /dev/null -w '%{http_code}\n' -X PUT --data x "$BASE/write-check"  # 403 or 405
curl -s -o /dev/null -w '%{http_code}\n' -X PUT --data x "$S3/write-check"    # 403
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE "$OBJECT"           # 403 or 405
```

A `200` for a listing shows the bucket's contents to anyone; one for a write lets anyone put
files at your media address.

## Upgrading

1. Build the new binaries and web client.
2. Run `aspen-migrate up`.
3. Restart the API servers, then the voice servers. Calls on a voice server that restarts end,
   and their clients rejoin on their own.
4. Copy the new web client over the old on each API server (see
   [Releasing a new web client](#releasing-a-new-web-client)).

People's event streams reconnect by themselves when an API server restarts, and pick up exactly
where they left off.
