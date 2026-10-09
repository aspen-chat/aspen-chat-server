# Installing: design notes

Why the operator's installation guide ([`docs/operators/installing/`](../../operators/installing/index.md))
asks what it does.

## Services

- **Valkey runs with `noeviction`.** An eviction policy would drop rate limit counts (letting
  guesses through) or sign-ins in progress at random. A full Valkey that refuses writes is
  instead treated as one that cannot be reached, so the endpoints that depend on it answer
  `serverBusy` until it has room. See [Step 1](../../operators/installing/1-services.md#valkey).

## Building

- **Both servers link the system's OpenSSL 3 rather than a copy built in.** Its security updates
  then reach them with the system's. See [Step 2](../../operators/installing/2-building.md).

## The deployment's address

- **`public_url` is chosen for good.** Its host is the federation domain and the domain passkeys
  belong to: other deployments pin the key they find there, and passkeys are bound to the host.
  See [Step 3](../../operators/installing/3-configuring.md#choosing-public_url).

## Outbound requests

- **Fetches to addresses others choose ignore `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`.** A
  proxy would resolve names itself, past the server's check that refuses private addresses. See
  [Step 4](../../operators/installing/4-api-server.md#outbound-requests).

## The web client

- **Every path the API does not own is answered with the web client's page.** The web client's
  links are real paths, such as `/communities/…`.
- **API answers carry `Cache-Control: no-store`** (but for the few meant to be kept), so no cache
  between keeps one user's answers.
- **The reverse proxy passes the server's headers through.** A second Content Security Policy is
  applied as well as the first, so one that leaves out the storage breaks pictures and uploads.
- **A new release is copied over the old rather than replacing the directory.** Browsers that
  already have the web client open load parts of it (the code highlighting for each language,
  the QR code reader) only when they need them, from the release they started with.

See [Step 5](../../operators/installing/5-web-client.md).

## Voice servers

- **Each voice server signs in to NATS as a user of its own.** It runs on a machine people send
  media to. The API servers' token lets whoever holds it read and write every event of the
  deployment, so a voice server gets only the subjects it needs.
- **Replies go under `_INBOX_voice.{id}`** rather than NATS's shared `_INBOX`, so a voice server
  can be allowed only its own replies.
- **The join token key is made by the first API server to start and kept in the database.** A
  voice server holds only the public half, which it asks for over `aspen.voice.token-key`, so it
  can check the tokens that let people into calls but never make one, and no secret needs
  copying to it.
- **A report is applied only when its subject names the server it is about, and the call or
  channel it is about is that server's.** A voice server taken over can then misreport its own
  calls and no one else's.
- **Registering gives a voice server an id that never changes while it stays registered.** The
  id is how the deployment tells its voice servers apart: reports, join tokens, and the NATS
  user's subjects all name it, so it is written once into `voice_server.toml` and the NATS
  permissions. `voice-servers add` prints it with those lines so an operator need not query the
  database.
- **A voice server checks its id against the registry at startup, and the API servers report
  unregistered ids they hear from.** A wrong id otherwise only shows as a Silent server, since its
  reports match no server and are dropped.
- **The API servers answer the join token key request only on the asking server's own inbox.**
  NATS lets a request name any subject for its reply, so a voice server taken over could have
  answers published on the API servers' subjects (making them reload plugins, redo background
  passes, or replace a rate limit suspension with an unreadable value). That cannot make anything
  it says believed, and refusing other reply subjects stops it; firewalling NATS remains the main
  defence.
- **When upgrading from shared-secret join tokens, voice servers go first**, while they still
  have the secret, so they take both kinds of token while the API servers move to the key.

See [Step 6](../../operators/installing/6-voice-servers.md) and
[Upgrading](../../operators/installing/upgrading.md#from-shared-secret-join-tokens).

## Network exposure and storage

- **Internal services stay off the internet because each trusts whoever reaches it.** NATS
  carries every event and can sign anyone into a call, Valkey holds the codes being mailed and
  the rate limits, and the storage's internals write without credentials.
- **`public_base_url` is served from an origin of its own**, never under `public_url`'s, so
  nothing posted can act as the deployment's pages.
- **The read path sends `X-Content-Type-Options: nosniff`**, so a browser opens a file only as the
  type it was stored as.
- **A password in a `redis://` address to another machine stops the server.** Without TLS the
  password and everything stored cross the network readable, so the server asks for `rediss://`
  instead of sending it.
- **`[nats.tls]` makes TLS required whatever `nats_url` names**, so no one between can strip it.

See [Network exposure](../../operators/installing/network-exposure.md) and
[The storage's read path](../../operators/installing/storage-read-path.md).
