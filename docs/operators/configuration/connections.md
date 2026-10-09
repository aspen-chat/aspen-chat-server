# Connections and events

How the server delivers events to connected clients, and what its listener admits.

## Event delivery

| Setting | Default | |
| --- | --- | --- |
| `event_queue_size` | `512` | How many events one connection may have waiting to be written. A connection that falls this far behind (a very slow network) is dropped, and its client reconnects and catches up. |
| `event_feed_shards` | one per CPU | How many tasks deliver events to this server's connections. |
| `event_retained_mib` | `256` | How much of the last minute's events, by the size of their text, this server keeps for catching reconnecting clients up. Past it the oldest are let go early, and a client that would have resumed from before them reloads its state instead. |

## `[connections]`

What the server's listener admits, and how long it waits for clients.

- A connection over a limit is closed as soon as it is accepted.
- The server logs that it is closing new connections at most once a minute.
- An event stream's connection counts for as long as it is open.
- A connection counts toward its address's and network's limits until someone signs in on it (a
  request presents a session, or its event stream identifies). From then on it counts toward that
  user's `max_per_user` instead, while they have room.

| Setting | Default | |
| --- | --- | --- |
| `max` | `100000` | The most connections the server holds open at once. Keep it below the process's [open file limit](#open-file-limit). |
| `max_per_ip` | `512` | The most one address holds open at once. See [Per-address limits](#per-address-and-per-network-limits). |
| `max_per_network` | `4096` | The most one network, an IPv4 /24 or an IPv6 /48, holds open at once, alongside `max_per_ip`. Raise it if many of your people share one carrier-grade NAT block. |
| `max_per_user` | `64` | The most connections one user holds open at once once signed in on them. A signed-in connection past it stays on its address's share rather than being closed. |
| `handshake_seconds` | `10` | How long a client has to finish its TLS handshake. |
| `header_read_seconds` | `30` | How long an HTTP/1.1 client has to send a request's headers. This is also how long a kept-alive connection may sit idle. |
| `idle_seconds` | `120` | How long a connection may stay open with no request in it before the server closes it. See [Idle connections](#idle-connections). |

### Open file limit

Keep `max` below the process's open file limit, with room for its connections to PostgreSQL,
NATS, Valkey, and storage:

- under systemd: `LimitNOFILE`
- in Docker: `--ulimit nofile`

### Per-address and per-network limits

- An IPv6 address counts toward `max_per_ip` by its `[rate_limits] ipv6_prefix` network.
- The address limits count only connections nobody has signed in on yet, so many people behind
  one address (an office, a school, a carrier-grade NAT) are each held to `max_per_user` rather
  than all to `max_per_ip`. Raise `max_per_ip` if so many arrive from one address at once, before
  signing in, that some are closed: the log says when.
- Reverse proxies listed in `[rate_limits] trusted_proxies` count only toward `max`, not toward
  `max_per_ip` or `max_per_network`.
- While the rate limits are suspended (`aspen-chat-server limits suspend`), connections from the
  suspension's networks, or from anywhere with `--scope all`, count only toward `max` too, so a
  load test's generators are not closed for holding many people's connections.
- So when every client arrives through one proxy, limit connections per client at the proxy.

### Idle connections

- An idle HTTP/1.1 connection is closed after `header_read_seconds`.
- An idle HTTP/2 connection is pinged every 30 seconds, and closed when a ping goes 20 seconds
  unanswered.
- A connection with no request in it for `idle_seconds` is closed. An HTTP/2 connection is sent
  GOAWAY. Clients open a new one when they next need it.
- An event stream's WebSocket is not an HTTP connection once it opens, and `idle_seconds` does
  not close it.
