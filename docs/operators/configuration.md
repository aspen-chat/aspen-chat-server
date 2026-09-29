# Configuration

The API server reads `aspen.toml` from its working directory. Any setting can be given in the
environment instead, which overrides the file: `ASPEN_` followed by the key, with `__` between a
section and its keys (`ASPEN_DATABASE_URL`, `ASPEN_FEDERATION__DOMAIN`,
`ASPEN_VOICE__IDLE_SESSION_SECONDS`). A setting left out takes the default shown. A value the
server cannot read stops it at startup with a message naming the setting; in `[federation]`, so
does a key it does not know, since a misspelt gate would otherwise leave it closed without a
word.

A voice server reads `voice_server.toml` the same way, with the prefix `ASPEN_VOICE_SERVER_`;
see [the voice server](#voice_servertoml) at the end.

## The services

| Setting | Default | |
| --- | --- | --- |
| `database_url` | required | PostgreSQL, as `postgres://user:password@host/database`. |
| `nats_url` | required | NATS with JetStream, as `host:4222`. |
| `nats_auth_token` | required | The token NATS was started with. |
| `valkey_url` | required | Valkey, as `redis://host:6379`. |

## Event delivery

| Setting | Default | |
| --- | --- | --- |
| `event_queue_size` | `512` | How many events one connection may have waiting to be written. A connection that falls this far behind (a very slow network) is dropped, and its client reconnects and catches up. |
| `event_feed_shards` | one per CPU | How many tasks deliver events to this server's connections. |

## `[media.s3]`

Where attachments, icons, avatars, and link preview images are kept. Clients upload straight to
storage with short-lived URLs the server signs, and download from a public path, so two of these
addresses are the clients' and must be reachable by them.

| Setting | Default | |
| --- | --- | --- |
| `endpoint` | `http://127.0.0.1:3900` | The S3 API, as this server reaches it. |
| `public_endpoint` | `endpoint` | The S3 API as clients reach it; the upload URLs they are handed name it. It must allow your web client's origin by CORS (`PUT`, with `Content-Type`). Leaving it out suits only clients on the server's own machine. |
| `public_base_url` | `http://127.0.0.1:3902/aspen-media` | Where clients download objects: a public read path on the bucket, such as a website endpoint or a CDN. The server itself never needs to reach it. |
| `bucket` | `aspen-media` | |
| `region` | `garage` | Whatever your storage expects; many accept any. |
| `access_key`, `secret_key` | development values | A key pair that may read, write, and delete in the bucket. |
| `upload_url_ttl_seconds` | `900` | How long an upload URL works. |

## `[cors]`

| Setting | Default | |
| --- | --- | --- |
| `allowed_origins` | `[]` | Page origins that may call the API from a browser, such as `["https://app.example.org"]`. Empty sends no CORS headers, which is right when the web client is served from the API's own origin. `["*"]` allows every origin, which is safe because Aspen authenticates with a header, never a cookie, but suits development only. A deployment whose [federation](federation.md) immigration gate admits anyone allows every origin regardless, since its visitors' web clients live elsewhere. |

## `[auth]`

| Setting | Default | |
| --- | --- | --- |
| `require_two_factor` | `false` | Every account must have an authenticator app or a passkey. An account without one can do nothing but add one or sign out. |
| `service_name` | `"Aspen"` | How authenticator apps and passkey prompts name this deployment. |
| `reverify_seconds` | `600` | Changing security settings needs a password or code given within this long. |
| `password_hashing_threads` | one per CPU | How many passwords may be hashed or checked at once. Each takes 19 MiB and most of a core for a moment. |
| `password_hashing_wait_seconds` | `10` | How long a sign-in waits for one of those threads before it is refused with `serverBusy`. |

### `[auth.passkeys]`

Passkeys are offered only when this section is present.

| Setting | | |
| --- | --- | --- |
| `rp_id` | required | The domain passkeys belong to, such as `chat.example.org`. Browsers offer a passkey only to pages on this domain or under it. **Changing it makes every registered passkey useless.** |
| `origins` | required | Every page origin that may use a passkey: this server's own (it serves the page the desktop and mobile apps open) and the web client's, if different, such as `["https://chat.example.org"]`. |

## `[registration]`

| Setting | Default | |
| --- | --- | --- |
| `invite_required` | `false` | Creating an account takes a registration invite, made in the dashboard or with `aspen-chat-server invites create`. |

## `[bots]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Whether people may make bots. Bots already made keep working either way. |
| `max_per_user` | `25` | The most bots one person may own. |

## `[limits]`

| Setting | Default | |
| --- | --- | --- |
| `max_communities_per_user` | `500` | The most communities one account may belong to. It bounds how much each connection reads and how far one profile change spreads. |

## `[presence]`

| Setting | Default | |
| --- | --- | --- |
| `away_after_seconds` | `600` | How long someone connected but not using Aspen stays shown as online before showing as away. Clients report use at most once a minute, so much less than a few minutes makes people flicker. |

## `[voice]`

| Setting | Default | |
| --- | --- | --- |
| `token_secret` | a development value | Signs the tokens that let people into calls; every voice server must have the same. **Set it to a long random string.** |
| `failure_threshold` | `5` | How many different people failing to reach a voice server, within `failure_window_seconds`, disable it until an administrator enables it again. |
| `failure_window_seconds` | `3600` | |
| `join_token_ttl_seconds` | `60` | How long someone has to reach a voice server after asking to join. |
| `candidate_limit` | `10` | The most voice servers one person is offered to choose the nearest from. |
| `offer_silence_seconds` | `60` | A voice server that has not reported for this long is not offered to people joining. |
| `session_silence_seconds` | `86400` | A voice server that has not reported for this long has its calls ended. Long on purpose: a call is worth more than tidiness after a brief network fault. |
| `idle_session_seconds` | `86400` | A call that never had two people in it at once ends after this long, so a forgotten client cannot hold a place on a voice server. |

### `[[voice.servers]]`

Voice servers to register at startup, one table each; they are matched by name, so changing a
`url` or `capacity` here updates the registered server. Administrators can also add and change
them in the dashboard.

| Setting | | |
| --- | --- | --- |
| `name` | required | |
| `url` | required | Where clients reach it, as `https://voice-1.chat.example.org`. |
| `capacity` | required | The most people it carries at once; `voice_server estimate-capacity` suggests one. |

## `[push]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Whether the Aspen app on phones may ask to be woken when it is not open, for DMs and messages that tag someone. Phones are woken through the relay of whoever published their app (the Aspen Foundation's, for the published apps) or through a UnifiedPush distributor; this server calls them over HTTPS, as it calls other deployments. What it sends them is encrypted to the phone, and says only which channel and message to fetch. |

## `[metrics]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Prometheus metrics at `GET /metrics` on a listener of their own. |
| `listen_addr` | `127.0.0.1:9464` | Keep it off public interfaces: the figures describe the deployment's inside. |

## `[rate_limits]`

Every endpoint is rate limited. The built-in limits, with what each is for, are in
`server/src/rate_limits.toml`, which also explains the format; what you write here is laid over
them, a limit you give replacing the built-in one whole.

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | |
| `trusted_proxies` | `[]` | Reverse proxies, as addresses or networks (`"10.0.0.0/8"`), whose `X-Forwarded-For` names the client. **Set it when the server is behind a proxy**, or every client counts as the proxy. |
| `ipv6_prefix` | `64` | IPv6 clients are counted by their network of this many bits, since one household holds a whole /64. |
| `max_suspension_seconds` | `86400` | The longest this server honours a suspension of the limits (`aspen-chat-server limits suspend`), counted from when it began. |

Limits for one endpoint go under its method and path:

```toml
[rate_limits.endpoints."POST /channels/{channel}/messages"]
user = { requests = 10, per_seconds = 10, burst = 5 }
```

A limit that names an endpoint, or a path parameter, that does not exist stops the server at
startup with a message saying which.

## `[federation]`

See [Federation](federation.md) for what these mean together.

| Setting | Default | |
| --- | --- | --- |
| `domain` | none | This deployment's name among deployments: the domain it is served at, with `:port` when not 443, such as `chat.example.org`. Required when any gate is not closed. **Other deployments remember the key they find at this name, so never change it.** |
| `standing_interval_seconds` | `3600` | How often this deployment asks other deployments whether their users here are still in good standing. |
| `standing_grace_seconds` | `86400` | How long another deployment may go unreached before its users' sessions here end. |

### `[federation.users]` and `[federation.bots]`

| Setting | Default | |
| --- | --- | --- |
| `emigration` | `"closed"` | Whether this deployment's accounts may use other deployments: `closed`, `open`, `allowList`, or `blockList`. |
| `immigration` | `"closed"` | Whether other deployments' accounts may use this one, the same way. |
| `shared_list` | `false` | Both directions read one list; both gates must then be the same kind of list. |
| `immigration_invite_required` | `false` | An account of another deployment arriving for the first time needs a registration invite. |

### `[federation.development]`

For running deployments side by side on one machine (`scripts/dev_federation.py`). Leave it out
of a deployment anyone else uses.

| Setting | Default | |
| --- | --- | --- |
| `extra_root_certificates` | `[]` | PEM files of certificate authorities to trust, besides the system's, when calling other deployments. |
| `allow_private_addresses` | `false` | Lets this server call deployments at private and loopback addresses, which it otherwise refuses so that naming a deployment cannot make it reach inside its own network. |

## `voice_server.toml`

| Setting | Default | |
| --- | --- | --- |
| `id` | required | The server's id in the registry (`SELECT id FROM voice_server WHERE name = '…'` once the API server has registered it). |
| `token_secret` | required | The API servers' `[voice] token_secret`. |
| `nats_url`, `nats_auth_token` | required | The same NATS as the API servers. |
| `listen_addr` | `0.0.0.0:9001` | Where the health check and signalling listen, as plain HTTP; put a TLS proxy in front. |
| `workers` | one per CPU | Media worker processes, each using at most one core. |

### `[rtc]`

| Setting | Default | |
| --- | --- | --- |
| `ip` | `0.0.0.0` | The interface media is received on. |
| `announced_address` | the primary interface's address | The address clients send media to. Set it to the public address when the server is behind NAT. Never a loopback address: the server refuses to start with one. |
| `min_port`, `max_port` | `40000`, `40999` | The media ports, UDP and TCP; open them to clients. The range bounds how many people the server can carry. |

### `[metrics]` and `[rate_limits]`

As the API server's, with `listen_addr` defaulting to `127.0.0.1:9465`. The voice server's
built-in limits are `voice_server/src/limits.toml`, which documents each, including
`max_message_bytes` and `max_pending_sockets_per_ip`.
