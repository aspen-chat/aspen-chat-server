# The voice server process

`voice_server/` is the media process: a mediasoup SFU. Each participant uploads once and the server forwards to everyone else. Its signalling is in [Signalling and media](signalling.md).

## Where it lives

| Piece | Code |
|---|---|
| Listener | `serve` in `voice_server/src/main.rs` |
| Rate limits | `voice_server/src/limits.rs`, built-in values in `voice_server/src/limits.toml` |
| Socket outbox | `voice_server/src/outbox.rs` |
| Token keys | `voice_server/src/token_keys.rs` |
| Capacity estimate | `voice_server/src/capacity.rs` |
| Seat claims | `SeatCounts` |
| Shared limit format, GCRA, trusted-proxy rule | `limits/` (`aspen_limits`) |

## HTTP endpoints

| Endpoint | Use |
|---|---|
| `GET /health` | Health check clients measure latency against. CORS open. |
| `GET /ws` | The signalling socket |

## Running it

- `cargo run -p voice_server`
- `cargo run -p voice_server -- --gen-signal-schema` writes `voice_signal_schema.json` (gitignored): the JSON Schema of every signalling frame, for the client's code generator.

## Configuration

It reads `voice_server.toml` from the working directory (gitignored). `ASPEN_VOICE_SERVER_` environment variables override it, using `__` for nesting.

| Key | Meaning |
|---|---|
| `id` | The server's row in the registry |
| `token_secret` | Only while upgrading (see [Join tokens](join-tokens.md#shared-secret-tokens)). Refused when it is the development value or shorter than 32 bytes, unless `development` is set. |
| `development` | Allows a weak `token_secret` |
| `nats_url` | The NATS address |
| `[nats_user]` | This server's own NATS user (see [NATS users](voice-reports.md#nats-users)), or `nats_auth_token` instead |
| `[nats]` | TLS to NATS (`[nats.tls]`) |
| `listen_addr` | HTTP listen address |
| `workers` | mediasoup workers, each at most one core |
| `[rtc]` | Media interface `ip`, `announced_address`, and the port range |
| `[transfer]` | STUN and TURN for file transfers (see [File transfers](../file-transfers.md)) |
| `[rate_limits]` | Laid over `limits.toml` the way the API server's are |
| `trusted_proxies` | Proxies whose forwarded address is honoured |
| `max_connections` | See [Connections](#connections) |
| `max_pending_sockets_per_ip`, `max_message_bytes` | See [Sockets](#sockets) |
| `max_seats_per_user`, `max_participants_per_call` | See [Seats and transports](#seats-and-transports) |

### The announced address

The announced address is what goes into ICE candidates.

- Set it for a server behind NAT.
- Left unset, with `ip` bound to every interface (the default), the host's primary interface address is announced. This suits a development machine.
- **It must never be a loopback address.** The server refuses to start with one. Firefox does not pair its own host candidates with a loopback peer, so signalling would succeed and media would never flow.

## Connections

| Limit | Value | Effect |
|---|---|---|
| `max_connections` | 10,000 | Connections held at once. More wait in the listen backlog until one closes. |
| `HEADER_READ_TIMEOUT` | ten seconds | Time to send each request's headers. Connections that send nothing or trickle headers are let go. |

## Address limits

- `GET /health` and opening `GET /ws` are limited per address, honouring `trusted_proxies`.
- A refusal is `429` with `Retry-After`.
- The server warns at startup when it listens on loopback (behind a proxy on its own machine) and trusts no proxies, since every client would then share the proxy's address.

## Sockets

| Limit | Value | Effect |
|---|---|---|
| `max_pending_sockets_per_ip` | eight | Sockets an address may hold that have not identified |
| `max_message_bytes` | 256 KiB | Largest message |
| `OUTBOX_BYTES` | 8 MiB | Frames waiting to be written to one socket. A socket whose client falls that far behind is closed. |
| `MAX_SIGNAL_BYTES` | 32 KiB | Largest transfer signal |
| Ping | every thirty seconds | A socket heard nothing from (pongs included) for a minute is closed |
| Closing | five seconds | Time a closing socket gets to write what is queued |

- The rooms queue frames without waiting on the socket. The outbox counts bytes since frames differ in size.
- `transferSignal`'s burst of sixty keeps one sender's frames well inside another's outbox.
- A client whose socket closes without a word rejoins.

## Frame rate limits

Each signalling frame type, and every frame together as `any`, is limited per `user`, `ip`, `channel` (the call), or `global`. The counters are in process, since every client of a call talks to this one server.

- A refused frame is dropped with a non-fatal `error`. It says how long to wait, in its text and as `retryAfterSeconds`, and names the frame's type as `refused`. Every non-fatal `error` answering a frame names it this way.
- A refused `identify` is a fatal `error`, named the same way.
- The limits let someone moving from call to call join a dozen times back to back, and every two seconds or so after that, as the API server's limit on join offers does.

### Muting is never limited

- A `setState` that mutes or deafens and lifts neither (`Rooms::quietens`) is never limited. Nothing, a flood of others' frames included, can keep someone's microphone open after they closed it.
- Only unmuting and undeafening count, per `user` alone. Each mute must be undone before it can be repeated, so that bounds both.
- The client disables its microphone's track the moment the user mutes, and shows them unmuted only once the server's `participantState` says so.

## Seats and transports

Every transport takes ports from the media range, so how many one account holds is bounded.

- A participant holds at most one producer per source (microphone, screen, screen audio), independently of rate.
- A participant holds one send transport. A second is refused while the first stands.
- A new receive transport replaces the last.
- A user is in at most `max_seats_per_user` (two) calls on a server at once (`SeatCounts`). Each participant holds a claim and passes it to whoever replaces them from another socket, so joining the same call again takes no more.
- A call holds at most `max_participants_per_call` (500).
- A join refused by either limit ends the room it started.
- `createTransport`, `produceRtp`, and `consumeRtp` are limited per address and for the whole server as well as per user.

### Connect timeout

A transport that has not connected within `CONNECT_TIMEOUT` (thirty seconds) is closed with what it carries, and the client is told with a non-fatal `error`. Connected means its DTLS handshake is done, or a plain transport's sender has been heard from.

| Transport | Closed with it |
|---|---|
| Send | Its producers |
| Receive | Its consumers, which a new receive transport gets again |
| Plain | Its producer |

The API server does not count seats itself. Its record of who is in which call follows the voice servers' reports, so the voice server is where the count is exact.

## Capacity

`voice_server estimate-capacity` gives the `capacity` to register the server with.

1. It reads the hardware: cores and any cgroup CPU quota, memory and any cgroup limit, the default route's link speed.
2. It reads the media settings: `workers` (each at most one core) and the RTC port range.
3. It measures on this CPU what a forwarded Opus and H.264 stream and a participant in whole calls cost, with a mediasoup worker on loopback for a few seconds.
4. It reports the smallest of the CPU, memory, bandwidth, and port limits after a safety margin.

The call shape is given by flags: `--call-size`, `--screen-share`, `--audio-kbps`, `--screen-kbps`, `--margin`, `--link-mbps`, `--json`.

### Synthetic H.264

mediasoup recognises an H.264 keyframe by its sequence parameter set, and a video consumer forwards nothing until it has seen one. Anything that sends synthetic H.264 (the calibration, the bench's simulated screens) leads each keyframe with an SPS and a PPS, as an encoder does.
