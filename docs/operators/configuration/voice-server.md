# Voice server

A voice server reads `voice_server.toml` from its working directory. The environment overrides
it, with the prefix `ASPEN_VOICE_SERVER_` (see [How settings are read](index.md#how-settings-are-read)).
Registering voice servers with the API servers is on [Voice calls](voice.md#voice-servers).

## `voice_server.toml`

| Setting | Default | |
| --- | --- | --- |
| `id` | required | The server's id in the registry, which `voice-servers add` prints and `voice-servers list` and the dashboard's Server fleet tab show. The server stops at startup when the API servers say no registered server has it. |
| `token_secret` | | Only while upgrading from API servers that signed join tokens with a shared secret. Leave it out otherwise. See [`token_secret`](#token_secret). |
| `development` | `false` | Lets the server start with a development or short `token_secret`. **Never on a deployment people use: whoever knows the secret can join any call.** |
| `nats_url` | required | The same NATS as the API servers; `tls://` over TLS. The server warns at startup when NATS on another machine is reached without TLS. |
| `[nats_user] user`, `password` | | This voice server's own NATS user, allowed only its own subjects. [Installing](../installing/6-voice-servers.md#give-it-a-nats-user) gives its permissions. |
| `nats_auth_token` | | The API servers' token instead, which lets this server do anything they can; the server warns at startup. Give exactly one of this and `[nats_user]`. |
| `[nats.tls]` | | As the API servers' [`[nats.tls]`](services.md#tls-to-the-services). |
| `listen_addr` | `0.0.0.0:9001` | Where the health check and signalling listen, as plain HTTP. Put a [TLS proxy](#behind-a-tls-proxy) in front. |
| `workers` | one per CPU | Media worker processes, each using at most one core. Run the server [under a supervisor](#workers). |

See [Voice servers](../installing/6-voice-servers.md) and
[upgrading from shared-secret join tokens](../installing/upgrading.md#from-shared-secret-join-tokens).

### `token_secret`

- Join tokens are signed with the API servers' key, which the voice server asks them for over
  NATS. Normally no `token_secret` is needed.
- While upgrading from API servers that signed join tokens with a shared secret, set it. The
  server then also takes tokens signed with it, and warns at startup.
- It refuses to start with the old development value, or one shorter than 32 bytes, unless
  `development` is set.

### Behind a TLS proxy

- Put a TLS proxy in front of `listen_addr`.
- List the proxy in `[rate_limits] trusted_proxies`, or every client counts as the proxy's
  address.
- The server warns at startup when `listen_addr` is a loopback address and no proxy is trusted.

### Workers

If one worker dies, the server stops, sends everyone in its calls to rejoin, and exits with an
error. Run it under a supervisor that restarts it: `Restart=on-failure` under systemd,
`restart: unless-stopped` in Docker Compose.

## `[rtc]`

| Setting | Default | |
| --- | --- | --- |
| `ip` | `0.0.0.0` | The interface media is received on. |
| `announced_address` | the primary interface's address | The address clients send media to. Set it to the public address when the server is behind NAT. Never a loopback address: the server refuses to start with one. |
| `min_port`, `max_port` | `40000`, `40999` | The media ports, UDP and TCP; open them to clients. The range bounds how many people the server can carry. |

## `[transfer]`

Files people offer each other in calls travel over a connection between their two devices,
encrypted end to end.

- The voice server answers STUN, so devices can find the addresses a direct connection would use.
- It relays transfers through TURN for people who choose not to connect directly, or whose
  connection cannot be made.
- It relays only between people in its own calls, never to the rest of the internet.
- It sees only ciphertext.

| Setting | Default | |
| --- | --- | --- |
| `relay_mbps` | `50` | The most every relayed transfer on this server may carry together, in megabits a second. People are told this limit before they choose the relay. `0` turns relaying off: transfers then go directly between devices or not at all. |
| `port` | `3478` | The UDP port STUN and TURN answer on; open it to clients. |
| `relay_min_port`, `relay_max_port` | `42000`, `42999` | The UDP ports relayed transfers use, two for each (one per side). They are used inside the server only and need not be open to clients. |

The server refuses to start when the relay ports overlap the media ports or `port`.

## `[metrics]` and `[rate_limits]`

As the API server's ([Metrics](metrics.md), [Rate limits](rate-limits.md)), with `listen_addr`
defaulting to `127.0.0.1:9465`.

The voice server's built-in limits are `voice_server/src/limits.toml`, which documents each,
including `max_message_bytes` and `max_pending_sockets_per_ip`. These bound how much of the
media port range one account can hold:

| Setting | Default | |
| --- | --- | --- |
| `max_connections` | `10000` | Connections the listener holds at once; more wait to be accepted. Raise the process's file limit (`ulimit -n`, `LimitNOFILE=`) above it. |
| `max_seats_per_user` | `2` | Calls one account may be in at once on this server. Joining the same call again from another device replaces the first and takes no more. |
| `max_participants_per_call` | `500` | People one call on this server may hold at once. |

Other limits:

- Every connection must send each request's headers within ten seconds.
- A transport that has not connected within thirty seconds of being made is closed.
- `createTransport`, `produceRtp`, and `consumeRtp` are limited per address and for the whole
  server, as well as per account.
- A load test from a few addresses suspends the limits (`limits suspend`).
