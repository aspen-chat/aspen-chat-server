# Voice server

A voice server reads `voice_server.toml` from its working directory. The environment overrides it,
with the prefix `ASPEN_VOICE_SERVER_` (see [How settings are read](index.md#how-settings-are-read)).
Registering voice servers with the API servers is on [Voice calls](voice.md#voice-servers).

## `voice_server.toml`

| Setting                        | Default                         |                                                                                                                                                                                                                         |
| ------------------------------ | ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                           | required                        | The server's id in the registry, which `voice-servers add` prints and `voice-servers list` and the dashboard's Server fleet tab show. The server stops at startup when the API servers say no registered server has it. |
| `token_secret`                 |                                 | Only while upgrading from API servers that signed join tokens with a shared secret. Leave it out otherwise. See [`token_secret`](#token_secret).                                                                        |
| `development`                  | `false`                         | Lets the server start with a development or short `token_secret`. **Never on a deployment people use: whoever knows the secret can join any call.**                                                                     |
| `nats_url`                     | required                        | The same NATS as the API servers; `tls://` over TLS. The server warns at startup when NATS on another machine is reached without TLS.                                                                                   |
| `[nats_user] user`, `password` |                                 | This voice server's own NATS user, allowed only its own subjects. [Installing](../installing/6-voice-servers.md#give-it-a-nats-user) gives its permissions.                                                             |
| `nats_auth_token`              |                                 | The API servers' token instead, which lets this server do anything they can; the server warns at startup. Give exactly one of this and `[nats_user]`.                                                                   |
| `[nats.tls]`                   |                                 | As the API servers' [`[nats.tls]`](services.md#tls-to-the-services).                                                                                                                                                    |
| `listen_addr`                  | `["0.0.0.0:9001", "[::]:9001"]` | Where the health check and signalling listen, as plain HTTP: one address or a list. Put a [TLS proxy](#behind-a-tls-proxy) in front. See [IPv4 and IPv6](#ipv4-and-ipv6).                                               |
| `media_threads`                | one per CPU                     | Threads that carry the calls' media, each using at most one core and taking one UDP and one TCP port from `[rtc] min_port` up. Run the server [under a supervisor](#media-threads).                                     |

See [Voice servers](../installing/6-voice-servers.md) and
[upgrading from shared-secret join tokens](../installing/upgrading.md#from-shared-secret-join-tokens).

### `token_secret`

- Join tokens are signed with the API servers' key, which the voice server asks them for over NATS.
  Normally no `token_secret` is needed.
- While upgrading from API servers that signed join tokens with a shared secret, set it. The server
  then also takes tokens signed with it, and warns at startup.
- It refuses to start with the old development value, or one shorter than 32 bytes, unless
  `development` is set.

### Behind a TLS proxy

- Put a TLS proxy in front of `listen_addr`.
- List the proxy in `[rate_limits] trusted_proxies`, or every client counts as the proxy's address.
- The server warns at startup when `listen_addr` is a loopback address and no proxy is trusted.

### IPv4 and IPv6

A voice server serves both families at once. Left to its defaults it listens on every interface over
IPv4 and IPv6, for signalling, media, and file transfers.

- Each of `listen_addr`, `[rtc] ip`, and `[rtc] announced_address` takes one value or a list:
  `ip = ["0.0.0.0", "::"]`. In an environment variable, separate them with commas, as every list is
  ([How settings are read](index.md#how-settings-are-read)):
  `ASPEN_VOICE_SERVER_RTC__IP="0.0.0.0,::"`.
- An IPv6 address takes IPv6 only, so `[::]:9001` and `0.0.0.0:9001` are two listeners and a
  client's address is always its own, never an IPv4 one written as IPv6.
- **A family you name must work.** The server refuses to start when an address in `listen_addr` or
  `[rtc] ip` cannot be bound, or has nothing to announce.
- **A family only the defaults name is left out when the machine lacks it**, with a line in the log
  (`no media over IPv6: …`). A machine without IPv6 runs on IPv4 alone with no setting changed.
- To serve one family only, name it: `listen_addr = "0.0.0.0:9001"` and `ip = "0.0.0.0"`.
- Open the same ports to clients over both families; an IPv6 firewall is usually a separate one
  (`ip6tables`, the cloud's IPv6 rules).

### Media threads

- Each call is carried by one media thread, so one call uses at most one core.
- Each thread receives media on one UDP port and one TCP port of every `[rtc] ip`: the first thread
  on `min_port`, the next on `min_port + 1`, and so on. Clients that cannot use UDP connect over
  TCP.
- If one media thread stops, the server stops, sends everyone in its calls to rejoin, and exits with
  an error. Run it under a supervisor that restarts it: `Restart=on-failure` under systemd,
  `restart: unless-stopped` in Docker Compose.
- Each thread's UDP socket asks for 4 MiB buffers each way. When the kernel allows less, the server
  warns at startup (`raise net.core.rmem_max (and wmem_max)`), and many people joining at once may
  lose packets. Raise them with `sysctl -w net.core.rmem_max=4194304 net.core.wmem_max=4194304`, and
  in `/etc/sysctl.d/` to keep it.

## `[rtc]`

| Setting                | Default                       |                                                                                                                                                                                                                                                                                       |
| ---------------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ip`                   | `["0.0.0.0", "::"]`           | The interfaces media is received on: one address, or one of each family.                                                                                                                                                                                                              |
| `announced_address`    | each family's primary address | The addresses clients send media to: one address, or one of each family, each used for the interface of its family. Set the public address when the server is behind NAT. Never a loopback address: the server refuses to start with one.                                             |
| `min_port`, `max_port` | `40000`, `40999`              | The media ports, UDP and TCP, the same over each family: `min_port` is the first media thread's, and each thread after it takes the next. The server refuses to start when `min_port` to `max_port` holds fewer ports than `media_threads`. Open `min_port` to `max_port` to clients. |
| `upload_mbps`          | none                          | The upload speed of the machine's internet connection, in Mbit/s, as a speed test run on it measures. `estimate-capacity` counts it in place of the network card's speed, which a connection at home is often far below. Nothing else reads it.                                       |

What a family announces when `announced_address` has no address of that family:

- An interface given by its own address (`ip = "203.0.113.10"`) announces that address.
- `0.0.0.0` and `::` announce the address the machine reaches the internet from in that family. For
  IPv6 that is often a temporary address that changes, so a server sets it explicitly:
  `announced_address = ["203.0.113.10", "2001:db8::10"]`.
- A host name in `announced_address` is announced for every family that has no address there.
  Browsers do not all resolve names in what a server announces, so prefer addresses.

## `[transfer]`

Files people offer each other in calls travel over a connection between their two devices, encrypted
end to end.

- The voice server answers STUN, so devices can find the addresses a direct connection would use.
- It relays transfers through TURN for people who choose not to connect directly, or whose
  connection cannot be made.
- It relays only between people in its own calls, never to the rest of the internet.
- It sees only ciphertext.

| Setting                            | Default          |                                                                                                                                                                                                                                     |
| ---------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `relay_mbps`                       | `50`             | The most every relayed transfer on this server may carry together, in megabits a second. People are told this limit before they choose the relay. `0` turns relaying off: transfers then go directly between devices or not at all. |
| `port`                             | `3478`           | The UDP port STUN and TURN answer on, on each `[rtc] ip`; open it to clients over both families.                                                                                                                                    |
| `relay_min_port`, `relay_max_port` | `42000`, `42999` | The UDP ports relayed transfers use, two for each (one per side). They are used inside the server only and need not be open to clients.                                                                                             |

The server refuses to start when the relay ports overlap the media ports or `port`.

## `[metrics]` and `[rate_limits]`

As the API server's ([Metrics](metrics.md), [Rate limits](rate-limits.md)), with `listen_addr`
defaulting to `127.0.0.1:9465`.

The voice server's built-in limits are `voice_server/src/limits.toml`, which documents each,
including `max_message_bytes` and `max_pending_sockets_per_ip`. These bound how much of the server
one account can hold:

| Setting                     | Default |                                                                                                                                                                                                    |
| --------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `max_connections`           | `10000` | Connections the listener holds at once; more wait to be accepted. Raise the process's file limit (`ulimit -n`, `LimitNOFILE=`) above it, with room for the media threads' TCP connections besides. |
| `max_seats_per_user`        | `2`     | Calls one account may be in at once on this server. Joining the same call again from another device replaces the first and takes no more.                                                          |
| `max_participants_per_call` | `500`   | People one call on this server may hold at once.                                                                                                                                                   |

Other limits:

- Every connection must send each request's headers within ten seconds.
- An offer a client has not answered within ten seconds, or a connection that has not connected
  within thirty seconds of its first offer, ends: a call's connection takes its person out of the
  call (`connectionFailed`), and their client rejoins.
- A TCP connection to a media port that carries nothing a connection takes for ten seconds is
  closed. Each media thread holds at most 4096 TCP connections.
- Signalling frames are limited by type, per account, and some per address or per call: `identify`,
  `resume` (per address, since it comes before anyone is known), `answer`, `send`, `stopSending`,
  `watch`, `silence`, `setReceiving`, `restartIce`, `setState` (only one that unmutes or undeafens
  counts), and the file transfer frames, with a limit on every frame together (`any`). `limits.toml`
  gives each.
- A load test from a few addresses suspends the limits (`limits suspend`).
