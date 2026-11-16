# The voice server process

`voice_server/` is the media process: a selective forwarding unit on str0m. Each participant uploads
once and the server forwards to everyone else. Its media runs on media threads (`voice_media`), its
signalling and rooms on Tokio; the signalling is in [Signalling and media](signalling.md).

## Where it lives

| Piece                                         | Code                                                                            |
| --------------------------------------------- | ------------------------------------------------------------------------------- |
| Listener                                      | `serve` in `voice_server/src/main.rs`                                           |
| Media threads                                 | `voice_media/` (`Media::start`, `src/thread.rs`, `src/net.rs`)                  |
| Synthetic peer                                | `voice_media::peer` (feature `peer`)                                            |
| Metrics                                       | `voice_server/src/metrics.rs`, names in `aspen_metrics::voice`                  |
| Rate limits                                   | `voice_server/src/limits.rs`, built-in values in `voice_server/src/limits.toml` |
| Socket outbox                                 | `voice_server/src/outbox.rs`                                                    |
| Token keys                                    | `voice_server/src/token_keys.rs`                                                |
| Capacity estimate                             | `voice_server/src/capacity.rs`                                                  |
| Seat claims                                   | `SeatCounts`                                                                    |
| Shared limit format, GCRA, trusted-proxy rule | `limits/` (`aspen_limits`)                                                      |

## HTTP endpoints

| Endpoint      | Use                                                      |
| ------------- | -------------------------------------------------------- |
| `GET /health` | Health check clients measure latency against. CORS open. |
| `GET /ws`     | The signalling socket                                    |

## Running it

- `cargo run -p voice_server`
- `cargo run -p voice_server -- --gen-signal-schema` writes `voice_signal_schema.json` (gitignored):
  the JSON Schema of every signalling frame, for the client's code generator.

## Configuration

It reads `voice_server.toml` from the working directory (gitignored). `ASPEN_VOICE_SERVER_`
environment variables override it, using `__` for nesting.

| Key                                               | Meaning                                                                                                                                                                        |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`                                              | The server's row in the registry                                                                                                                                               |
| `token_secret`                                    | Only while upgrading (see [Join tokens](join-tokens.md#shared-secret-tokens)). Refused when it is the development value or shorter than 32 bytes, unless `development` is set. |
| `development`                                     | Allows a weak `token_secret`                                                                                                                                                   |
| `nats_url`                                        | The NATS address                                                                                                                                                               |
| `[nats_user]`                                     | This server's own NATS user (see [NATS users](voice-reports.md#nats-users)), or `nats_auth_token` instead                                                                      |
| `[nats]`                                          | TLS to NATS (`[nats.tls]`)                                                                                                                                                     |
| `listen_addr`                                     | HTTP listen addresses, one or a list; `0.0.0.0:9001` and `[::]:9001` by default                                                                                                |
| `media_threads`                                   | Media threads, each at most one core; one per logical CPU by default. See [Media threads](#media-threads)                                                                      |
| `[rtc]`                                           | Media interfaces `ip`, their `announced_address`es, `min_port` (the first thread's port, 40000) and `max_port` (the highest a thread may take, 40999), and `upload_mbps`       |
| `[metrics]`                                       | `enabled` and `listen_addr` (`127.0.0.1:9465`); see [Metrics](#metrics)                                                                                                        |
| `[transfer]`                                      | STUN and TURN for file transfers (see [File transfers](../file-transfers.md))                                                                                                  |
| `[rate_limits]`                                   | Laid over `limits.toml` the way the API server's are                                                                                                                           |
| `trusted_proxies`                                 | Proxies whose forwarded address is honoured                                                                                                                                    |
| `max_connections`                                 | See [Connections](#connections)                                                                                                                                                |
| `max_pending_sockets_per_ip`, `max_message_bytes` | See [Sockets](#sockets)                                                                                                                                                        |
| `max_seats_per_user`, `max_participants_per_call` | See [Seats and connections](#seats-and-connections)                                                                                                                            |

### The announced address

The announced address is what goes into ICE candidates. `RtcConfig::media_addresses` pairs each
interface of `[rtc] ip` with one, as a `MediaAddress`.

- `ip` holds at most one address of each family, and `announced_address` at most one address of each
  family and one host name.
- An interface announces the `announced_address` of its family, else the host name, else itself. An
  unspecified interface (`0.0.0.0`, `::`) with neither announces the address the host reaches the
  internet from in that family (`local_ip_address`), unless that is loopback or link-local.
- A host name is looked up once, before the media threads start, for an address of the interface's
  family (`MediaAddress::resolved`). One with no address of that family stops the server.
- Set it for a server behind NAT.
- **It must never be a loopback address.** The server refuses to start with one. Firefox does not
  pair its own host candidates with a loopback peer, so signalling would succeed and media would
  never flow.

### IPv4 and IPv6

Left unset, `listen_addr` and `[rtc] ip` name every interface over both families.

| Listener            | Per family                                                                                                                            |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| HTTP and signalling | One TCP listener for each `listen_addr`; `serve` accepts from them in turn                                                            |
| Media threads       | A UDP socket and a TCP listener on each media address, so every connection has candidates of both families and the client's ICE picks |
| STUN and TURN       | One UDP socket for each media address; see [File transfers](../file-transfers.md#stun-and-turn)                                       |

- Every IPv6 socket takes IPv6 alone (`aspen_sockets`). A port then has an IPv4 and an IPv6 socket,
  and a client's address is never an IPv4-mapped one.
- A family the settings name must work: an `ip` that cannot be bound or has nothing to announce, or
  a `listen_addr` that cannot be bound, stops the server.
- A family only the defaults name is left out, with a log line, when the machine lacks it
  (`aspen_sockets::family_missing`, or no address to announce). Any other failure to bind, a port in
  use for one, stops the server.

## Media threads

`voice_media` runs `media_threads` threads, named `aspen-media-0` onwards. Each holds the str0m
connections of the calls given to it, round robin; every participant of a call is on its call's
thread, so forwarding never crosses threads.

| Per thread  | What                                                                                                  |
| ----------- | ----------------------------------------------------------------------------------------------------- |
| Port        | `[rtc] min_port` plus the thread's index, over UDP and TCP, on every media address                    |
| UDP         | One socket per media address, asking for 4 MiB of buffer each way (`UDP_BUFFER_BYTES`)                |
| TCP         | One listener per media address; each accepted stream carries ICE, DTLS, and SRTP framed per RFC 4571  |
| Connections | Every participant's call connection, and their capture connection while the game capture helper sends |

- An operator opens as many ports as there are threads, over UDP and TCP, not a range sized to
  people. The server refuses to start when `min_port` to `max_port` holds fewer ports than threads,
  and when a port cannot be bound.
- The server warns at startup when the kernel gives a UDP socket less buffer than it asked for
  (`net.core.rmem_max`): a burst of joins (many DTLS handshakes at once) would drop packets.
- A thread waits in `poll` on its sockets, a waker the rooms write to with each command, and the
  earliest timeout any connection wants, kept in a heap so that a pass wakes only those due.
- A datagram goes to the connection its source address was paired with, or, the first time, to the
  one whose ICE username fragment its STUN request names, found in a map rather than by a scan.
- TCP streams are bounded: `MAX_TCP_STREAMS` (4,096) per thread, one whose client falls 1 MiB behind
  is dropped, and one that no connection has taken a packet of within `TCP_UNIDENTIFIED_SECONDS`
  (ten) is closed.
- str0m runs in RTP mode with ICE lite, its DTLS and SRTP through AWS-LC. The voice server links no
  OpenSSL.
- Every call into str0m for a connection is guarded (`catch_unwind`): a panic there drops that
  connection alone (`Failure::Internal`). Anything else that ends a thread stops the server
  (`Stopped::MediaThread`; see [Signalling](signalling.md#a-media-thread-stopping)).
- The rooms command a thread (`voice_media::Command`) and hear back from it (`voice_media::Event`):
  offers, a connection up or failed, a source's first media, speaking, and slot assignments.

## Connections

| Limit                 | Value       | Effect                                                                                            |
| --------------------- | ----------- | ------------------------------------------------------------------------------------------------- |
| `max_connections`     | 10,000      | Connections held at once. More wait in the listen backlog until one closes.                       |
| `HEADER_READ_TIMEOUT` | ten seconds | Time to send each request's headers. Connections that send nothing or trickle headers are let go. |

## Address limits

- `GET /health` and opening `GET /ws` are limited per address, honouring `trusted_proxies`.
- A refusal is `429` with `Retry-After`.
- The server warns at startup when it listens on loopback (behind a proxy on its own machine) and
  trusts no proxies, since every client would then share the proxy's address.

## Sockets

| Limit                        | Value                | Effect                                                                                             |
| ---------------------------- | -------------------- | -------------------------------------------------------------------------------------------------- |
| `max_pending_sockets_per_ip` | eight                | Sockets an address may hold that have not identified                                               |
| `max_message_bytes`          | 256 KiB              | Largest message                                                                                    |
| `OUTBOX_BYTES`               | 8 MiB                | Frames waiting to be written to one socket. A socket whose client falls that far behind is closed. |
| `MAX_SIGNAL_BYTES`           | 32 KiB               | Largest transfer signal                                                                            |
| Ping                         | every thirty seconds | A socket heard nothing from (pongs included) for a minute is closed                                |
| Closing                      | five seconds         | Time a closing socket gets to write what is queued                                                 |

- The rooms queue frames without waiting on the socket. The outbox counts bytes since frames differ
  in size.
- Frames are compressed with permessage-deflate (RFC 7692, `yawc`) where the client offers it, which
  every browser does: an offer deflates to about a ninth. Neither side keeps deflate state between
  messages, a socket borrows what it compresses with from one pool for the server (`DeflatePool`,
  one per logical CPU), and frames under 256 bytes go as they are (`COMPRESS_FROM_BYTES` in
  `voice_server/src/signalling.rs`). `max_message_bytes` bounds a message after it is inflated as
  well as before.
- `transferSignal`'s burst of sixty keeps one sender's frames well inside another's outbox.
- A participant whose socket closes without `leave`, closed by either side, stays in the call for
  the resume grace (see [Resuming](signalling.md#resuming)).

## Frame rate limits

Each signalling frame type, and every frame together as `any`, is limited per `user`, `ip`,
`channel` (the call), or `global`. The counters are in process, since every client of a call talks
to this one server.

- A refused frame is dropped with a non-fatal `error`. It says how long to wait, in its text and as
  `retryAfterSeconds`, and names the frame's type as `refused`. Every non-fatal `error` answering a
  frame names it this way.
- A refused `identify` is a fatal `error`, named the same way.
- A frame that is a JSON object but none this server knows (a type, or a value in a known type, that
  a newer client sends) is answered the same way: a non-fatal `error` naming its `type` as
  `refused`, and the socket stays open. It counts toward `any` alone. Before `identify`, and for
  text that is not a JSON object or a binary message, the socket is closed.
- The limits let someone moving from call to call join a dozen times back to back, and every two
  seconds or so after that, as the API server's limit on join offers does.
- `resume` is limited by address alone, before its secret is looked up, so secrets cannot be guessed
  at speed. A refused `resume` is a fatal `error`.

| Frame             | Built-in limit, per user unless named                                                  |
| ----------------- | -------------------------------------------------------------------------------------- |
| `any`             | 50 a second, bursts of 400                                                             |
| `identify`        | 30 a minute, bursts of 12                                                              |
| `resume`          | 30 a minute per address, bursts of 10                                                  |
| `answer`, `watch` | 120 a minute, bursts of 40 and 30: each change of what is sent or watched renegotiates |
| `send`            | 40 a minute, bursts of 20                                                              |
| `stopSending`     | 30 a minute, bursts of 10                                                              |
| `setReceiving`    | 600 a minute, bursts of 100: it changes forwarding alone                               |
| `restartIce`      | 20 a minute, bursts of 6                                                               |
| `setState`        | 40 a minute, bursts of 20, counting only what unmutes or undeafens                     |

The file transfer frames' limits are in `limits.toml` beside these, which documents each.

### Muting is never limited

- A `setState` that mutes or deafens and lifts neither (`Rooms::quietens`) is never limited.
  Nothing, a flood of others' frames included, can keep someone's microphone open after they closed
  it.
- Only unmuting and undeafening count, per `user` alone. Each mute must be undone before it can be
  repeated, so that bounds both.
- The client disables its microphone's track the moment the user mutes, and shows them unmuted only
  once the server's `participantState` says so.

## Seats and connections

Every participant holds a connection on a media thread, so how many one account holds is bounded.

- A participant sends at most one of each source (microphone, camera, screen, screen audio),
  independently of rate.
- A participant holds one call connection, and one capture connection while the helper sends for
  them.
- What one participant receives is bounded whatever the call's size: at most `AUDIO_SLOTS` (16)
  audio slots, and at most `MAX_WATCHED_VIDEO` (25) watched videos, a watched screen's sound beside
  it.
- A user is in at most `max_seats_per_user` (two) calls on a server at once (`SeatCounts`). Each
  participant holds a claim and passes it to whoever replaces them from another socket, so joining
  the same call again takes no more.
- A call holds at most `max_participants_per_call` (500).
- A join refused by either limit ends the room it started.

The API server does not count seats itself. Its record of who is in which call follows the voice
servers' reports, so the voice server is where the count is exact.

### Timeouts

| Limit             | Value          | Effect                                                                               |
| ----------------- | -------------- | ------------------------------------------------------------------------------------ |
| `ANSWER_TIMEOUT`  | ten seconds    | An offer unanswered this long fails its connection                                   |
| `CONNECT_TIMEOUT` | thirty seconds | A connection whose ICE and DTLS have not completed this long after it was made fails |
| Resume grace      | twenty seconds | A participant whose socket closed without `leave` waits this long to be resumed      |

A failed call connection takes its participant out of the call (`connectionFailed`), and their
client rejoins; a failed capture connection stops what the helper sends. See
[Signalling](signalling.md#a-connection-failing).

## Metrics

Served by `[metrics]` on a listener of its own (`127.0.0.1:9465`), sampled every
`aspen_metrics::SAMPLE_INTERVAL`:

| Metric                                                                                              | What                                                                       |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `aspen_voice_rooms`                                                                                 | Calls on this server                                                       |
| `aspen_voice_participants`                                                                          | Participants on the media threads                                          |
| `aspen_voice_connections`                                                                           | Peer connections: one per participant, one more per capture helper sending |
| `aspen_voice_sections`                                                                              | Media sections across every connection: sent, audio slots, watched video   |
| `aspen_voice_media_thread_cpu_seconds`                                                              | CPU seconds each media thread has used, by `thread` (`aspen-media-N`)      |
| `aspen_voice_frames_total`                                                                          | Signalling frames handled, by `kind`                                       |
| `aspen_voice_frames_refused_total`                                                                  | Frames refused for going too fast, by `kind`                               |
| `aspen_voice_http_refused_total`                                                                    | HTTP requests refused for going too fast, by `route`                       |
| `aspen_voice_rate_limits_suspended`                                                                 | 1 while a suspension of the rate limits is in force                        |
| `aspen_voice_transfers`, `aspen_voice_relayed_bytes_total`, `aspen_voice_relay_dropped_bytes_total` | File transfers; see [File transfers](../file-transfers.md)                 |

The census is counted by each media thread (`Command::Census`); a thread's CPU time is read from its
entry under `/proc/self/task`. jemalloc's figures (`aspen_memory_*`) are exported as on the API
server.

## Capacity

`voice_server estimate-capacity` gives the `capacity` to register the server with. It reads only the
media settings, so it runs on a machine not yet set up as a voice server.

1. It reads the hardware: cores and any cgroup CPU quota, memory and any cgroup limit, and the
   bandwidth out (`--link-mbps`, else `[rtc] upload_mbps`, else the default route's link speed).
2. It reads `media_threads`, each at most one core, and warns when there are fewer than cores.
3. It measures on this CPU, on a media thread of its own on loopback, for a few seconds each: audio
   streams (calls of six, each microphone forwarded to the five others), video streams (calls of
   six, one sharing a screen the others watch), and whole calls of the shape. The participants are
   `voice_media::peer` clients in the same process, each a full WebRTC connection (ICE, DTLS, SRTP
   both ways, RTCP); only the media thread's CPU time and allocations are counted.
4. It reports the smallest of the CPU, memory, and bandwidth limits after a safety margin, and the
   largest call one media thread holds, since a call lives on one thread.

It stops without a figure when a measurement finds the media thread used no CPU time: its time is
read from the thread named `aspen-media-…` under `/proc/self/task`, and a cost of nothing would
leave CPU out of the limits. It warns when fewer than 95% of a calibration's packets arrived.

The call shape is given by flags: `--call-size`, `--screen-share`, `--audio-kbps`, `--screen-kbps`,
`--margin`, `--link-mbps`, `--calibration-seconds`, `--json`.

### Synthetic H.264

A video section forwards nothing until it has seen a keyframe, which the media thread recognises by
an IDR slice or a sequence parameter set (`voice_media/src/keyframe.rs`). The synthetic peer, which
the calibration, the bench, and the tests send with, leads each second's keyframe with an SPS and a
PPS, as an encoder does.
