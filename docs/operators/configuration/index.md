# Configuration

Every setting of `aspen.toml` (the API server) and `voice_server.toml` (a voice server), and the
deployment settings kept in the database. Why the settings are shaped as they are is in the
[design notes](../../architecture/operations/configuration-design-notes.md).

## How settings are read

1. The API server reads `aspen.toml` from its working directory.
2. Any setting can instead be given in the environment, which overrides the file.
   - Write `ASPEN_` followed by the key.
   - Put `__` between a section and its keys.
   - Examples: `ASPEN_PUBLIC_URL`, `ASPEN_DATABASE_URL`, `ASPEN_VOICE__IDLE_SESSION_SECONDS`.
3. A setting left out takes the default shown on its page.
4. A value the server cannot read stops it at startup, with a message naming the setting.
5. In `[federation]`, `[web_client]`, and `[email]`, a key the server does not know also stops it.

A voice server reads `voice_server.toml` the same way, with the prefix `ASPEN_VOICE_SERVER_`. See
[`voice_server.toml`](voice-server.md#voice_servertoml).

## What is in the file and what is in the database

- `aspen.toml` holds what a server needs to start: where its services are, its secrets, its
  sizes, and what is bound to its domain.
- The database holds what the deployment's administrators decide: its name, who may register,
  second factors, bots, community limits, files in calls, and the federation gates. Every server
  follows one answer, and a change takes effect at once without a restart. See
  [Deployment settings](deployment-settings.md#deployment-settings).
- The voice servers calls run on are in the database too. See
  [Voice servers](voice.md#voice-servers).

## Sections

### `aspen.toml`

| Section | Page | What it sets |
| --- | --- | --- |
| `public_url` | [Address and web client](address.md#the-deployments-address) | The deployment's one address. **Can never change once federating.** |
| `[web_client]` | [Address and web client](address.md#web_client) | Where the built web client is. |
| `database_url`, `database_pool_*`, `nats_*`, `[nats_user]`, `[nats.tls]`, `valkey_url`, `[valkey.tls]` | [Services](services.md#the-services) | PostgreSQL, NATS, and Valkey. |
| `event_queue_size`, `event_feed_shards`, `event_retained_mib` | [Connections](connections.md#event-delivery) | Delivering events to connected clients. |
| `[connections]` | [Connections](connections.md#connections) | What the listener admits, and timeouts. |
| `[auth]` | [Accounts](accounts.md#auth) | Reverification and password hashing. |
| `[limits]` | [Accounts](accounts.md#limits) | Communities per account, event streams per account and address. |
| `[presence]` | [Accounts](accounts.md#presence) | When people show as away. |
| `[media]` | [Media](media.md#media) | Attachment size and served types. |
| `[media.s3]` | [Media](media.md#medias3) | Object storage. |
| `[media.s3.tls]` | [Media](media.md#medias3tls) | Authorities to trust for the storage's `endpoint`. |
| `[media.previews]` | [Previews](previews.md#mediapreviews) | Previews of pictures and videos' posters. |
| `[voice]` | [Voice](voice.md#voice) | Voice server failures, join tokens, and idle calls. |
| `[plugins]` | [Plugins](plugins.md#plugins) | Plugin time, memory, concurrency, notice, and storage limits. |
| `[jobs]` | [Jobs](jobs.md#jobs) | Whether this server runs background jobs, and how many at once. |
| `[push]` | [Push](push.md#push) | Waking phones. |
| `[email]` | [Email](email.md#email) | Sending mail. |
| `[email.tls]` | [Email](email.md#emailtls) | Authorities and a client certificate for the SMTP server. |
| `[metrics]` | [Metrics](metrics.md#metrics) | The Prometheus endpoint. |
| `[rate_limits]` | [Rate limits](rate-limits.md#rate_limits) | Rate limits and trusted proxies. |
| `[federation]` | [Federation](federation.md#federation) | Standing checks and arrivals. |
| `[federation.development]` | [Federation](federation.md#federationdevelopment) | Deployments side by side on one machine. |

### In the database

| What | Page |
| --- | --- |
| Deployment settings (`aspen-chat-server settings`) | [Deployment settings](deployment-settings.md) |
| Voice servers (`aspen-chat-server voice-servers`) | [Voice](voice.md#voice-servers) |

### `voice_server.toml`

| Section | Page |
| --- | --- |
| Top-level settings | [Voice server](voice-server.md#voice_servertoml) |
| `[rtc]` | [Voice server](voice-server.md#rtc) |
| `[transfer]` | [Voice server](voice-server.md#transfer) |
| `[metrics]` and `[rate_limits]` | [Voice server](voice-server.md#metrics-and-rate_limits) |
