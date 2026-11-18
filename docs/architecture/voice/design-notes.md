# Voice design notes

Why the voice pieces are shaped as they are. The how is in the [voice pages](index.md).

## Registry

- **`remove` refuses a server that holds calls.** Ending them takes a server's context.
  ([Servers](servers.md))
- **`add` also updates a server of the same name.** A deployment's scripts may then run it on every
  deploy. ([Servers](servers.md))

## Join tokens

- **Tokens are signed with an Ed25519 key, and voice servers hold only the public half.** A voice
  server can check tokens but never make one. ([Join tokens](join-tokens.md))
- **The public half is fetched over NATS from the API servers' queue group.** Only the API servers
  may publish to a voice server's inbox, so the answer is theirs. Asking again for an unknown key is
  capped at once every five seconds.
  ([Join tokens](join-tokens.md#how-a-voice-server-gets-the-public-half))
- **The key request names the voice server's id and the answer says whether it is registered.** A
  voice server given the wrong id stops at startup rather than reporting to no one. The id is only
  asked about, never believed. ([Join tokens](join-tokens.md#checking-the-voice-servers-id))
- **Registration look-ups are capped at `REGISTRATION_LOOKUPS` (two) per API server.** A voice
  server asking over and over holds at most that many database connections.
- **Key answers go only to the asking server's inbox.** Any other reply subject is a sign of a
  compromised voice server. ([Join tokens](join-tokens.md#how-a-voice-server-gets-the-public-half))
- **Shared-secret tokens are still accepted under `token_secret`.** A deployment can then upgrade
  from API servers that made them without breaking calls.
  ([Join tokens](join-tokens.md#shared-secret-tokens))
- **A token admits one connection to each server it names, tracked by nonce.** Someone removed
  cannot come back on the token they joined with.
  ([Join tokens](join-tokens.md#one-connection-per-token))

## Sources

- **A source is sent only on a section the server offered for it, of its source's kind.** What may
  be sent and what a mute pauses are decided by source, so a source must mean what it says.
  ([Rechecks](rechecks.md#what-the-voice-server-enforces-on-each-source))
- **Grants are checked at `send` and again when the source's first media arrives.** Between the two
  the media thread sets the section up and the client answers; a source whose grant went meanwhile
  is stopped before anyone is told of it. ([Rechecks](rechecks.md))
- **Others are told of a source only once its media arrives.** A client that asked to send and never
  did shows nothing to the call. ([Signalling](signalling.md#sending))
- **One of each source per participant.** No client can multiply what everyone else in the call
  receives. ([The voice server process](voice-server.md#seats-and-connections))

## Rechecks

- **Rechecks run after the work commits, from `rechecks_of`, which matches every event with no
  wildcard.** No call site needs to remember them, and a new event cannot be published until its
  effect on calls is decided. ([Rechecks](rechecks.md))
- **Rechecks run on their own tasks, at most `RECHECKS_AT_ONCE` (16) at a time per server, and a
  recheck of every call runs as a job.** Turning file transfers on or off reaches every call on the
  deployment, so `recheckAllCalls` takes a batch of seats at a time.
  ([Rechecks](rechecks.md#when-a-recheck-runs))
- **A join's report rechecks the joiner, and is checked against live sign-ins.** A token issued
  before a change, or before its sign-in ended, and used after it, is brought in line once the join
  is recorded. ([Rechecks](rechecks.md#ended-sign-ins))
- **A recheck always sends `Mute` with the mute as it stands, and the voice server ignores no-ops.**
  A token issued before a change is brought in line once its join is recorded.
  ([Moderation](moderation.md#how-it-reaches-the-call))
- **Every snapshot rechecks everyone in the call.** Commands go over core NATS with no
  acknowledgement, so a lost kick, grant, or mute has nothing else to repair it; the voice server
  ignores a command that changes nothing, so the recheck costs the call's size once a minute and
  changes nothing when nothing was lost. Acknowledged commands would need a reply from the voice
  server and a retry on each API server for every command in flight.
  ([Snapshots](snapshots.md#what-the-api-server-does-with-it))

## Server selection

- **Candidates are chosen at random for now**, at most `candidate_limit`; the client picks among
  them by measured latency. ([Choosing a server](server-selection.md))
- **A server silent for `offer_silence_seconds` is not offered.** It is probably down, and offering
  it would cost clients failed attempts and count against it.

### Failure reports

- **A report counts only from someone offered the server who has not joined a call there within the
  window.** No one can suspend a server they were never sent to or reached.
  ([Choosing a server](server-selection.md#which-reports-count))
- **Bots and foreign users' reports do not count.** A bot's owner may have many bots; a foreign user
  is vouched for by another deployment.
- **A Valkey outage is read as having joined.** Reports then count for nothing rather than for
  anyone.
- **A server is never suspended when no other would be left taking calls, decided with every row
  locked.** Throwaway accounts can at worst suspend every server but one.
- **A suspension ends on its own after the window.** By then the server's failures are all older
  than the window, so it is offered again without an operator.

## Reports

- **Reports are partitioned by channel into lanes, each with one shared durable consumer handing out
  one report at a time.** The reports about one channel are applied once and in order by whichever
  API server takes them, while lanes are applied side by side, so a busy channel holds up only its
  lane. ([Voice reports](voice-reports.md#lanes))
- **Speaking changes have lanes of their own.** They come many times faster than anything else, and
  never hold up joins and leaves.
- **Report application is capped at half the database pool.** Requests always have the other half.
- **A report's source is the server its subject names, never what it says.** With per-server NATS
  users, a voice server taken over can misreport its own calls and no one else's.
  ([Voice reports](voice-reports.md#where-a-report-came-from))
- **A new call or participant needs an offer note.** A voice server taken over cannot make up a
  call, or someone in one, anywhere it was not sent, to ring a DM or show a call that is not there.
- **A Valkey outage believes the report.** Calls go on through it.
- **Applying a report is idempotent.** A report whose acknowledgement was lost is delivered again.
- **Reply subjects can still be aimed elsewhere.** NATS checks a reply subject against no one's
  permissions, so a voice server taken over can have the JetStream API publish on the API servers'
  subjects; none of that is believed as a report, a command, or an event. Only a NATS account of the
  voice servers' own would keep replies within it.
  ([Voice reports](voice-reports.md#what-a-voice-server-taken-over-can-still-cause))
- **Where each session is, is remembered for ten seconds for speaking changes.** A session's server
  and channel never change, so a lively call reads the database once per call every ten seconds
  rather than once per change. ([Voice reports](voice-reports.md#speaking-changes))
- **A load report from an unregistered id updates nothing.** It is noted for the dashboard instead.
- **`load` records when the stream took the report.** A report that waited in the stream does not
  make a dead server look alive.

## Snapshots

- **Voice servers send snapshots every minute and on reconnecting.** Reports can be lost while NATS
  is unreachable, when NATS restarts, or by being dropped. ([Snapshots](snapshots.md))
- **A snapshot's parts are queued together, each after every earlier report about its calls.** Each
  part then describes things as of its place in the order.
- **Each repair logs a warning.** One means a report was lost.
- **A server's first snapshot waits a full interval.** The calls it held before a restart are
  carried on by their people rejoining rather than ended first.

## Sessions

- **A join offer ends the call of a server that stopped reporting.** The people of a dead server's
  call rejoin within seconds, and anyone else joining its channel is the next to notice; waiting for
  the reaper would leave the call on record, its people shown in it and its commands going to no
  one, for `session_silence_seconds`. The offer passes the lost call's start and starter on through
  Valkey, so the call that goes on keeps its record and rings no one again, as it does when
  `record_session` finds the stale row itself. ([Sessions](sessions.md#when-a-server-dies))
- **`session_silence_seconds` is fifteen minutes.** Minutes rather than seconds, so a brief outage
  of the report link (NATS restarting, a voice server reconnecting) does not end anyone's call,
  since the snapshot that follows repairs what was lost; minutes rather than hours, since the calls
  of a dead server that nobody tries to join stay on record until the reaper ends them.
  ([Sessions](sessions.md#silence-and-the-reaper))
- **`idle_session_seconds` ends calls that never held two people.** A forgotten client cannot hold a
  voice server slot.
- **Ending a call locks its session row.** The reaper and a report, ending one call at once, record
  the end once.
- **A duplicate room on another live server is closed, not recorded.** Join tokens name up to ten
  servers, so two rooms can open at once; the recorded call wins and the other's people are sent to
  it. ([Sessions](sessions.md#duplicate-rooms))
- **Rejoining waits a random delay of at most a second.** The participants of a lost call do not all
  hit the API server in the same instant.

## Moderation

- **The participant endpoints answer `202 Accepted`.** The voice server applies the command and its
  report changes the record, so the response is not the result. ([Moderation](moderation.md))
- **A server mute is the community's and outlives leaving.** Leaving and coming back does not lift
  it.

## The voice server

- **The announced address may never be loopback.** Firefox does not pair its own host candidates
  with a loopback peer, so signalling would succeed and media would never flow.
  ([The voice server process](voice-server.md#the-announced-address))
- **Both address families by default, each on a socket of its own.** A client on an IPv6-only
  network (many phones) reaches a call without a translator in the path, and one with both lets ICE
  pick. One `::` socket taking IPv4 too would show clients at IPv4-mapped addresses, which the
  address limits and `trusted_proxies` would have to see through, and would depend on the machine's
  `bindv6only`. ([The voice server process](voice-server.md#ipv4-and-ipv6))
- **A family the defaults name is given up when the machine lacks it; one the settings name is
  not.** A machine with no IPv6 starts unchanged, and an operator who asked for a family learns at
  startup that it is not there, rather than from the people who could not connect.
- **Only a missing family is given up, never a port in use.** Otherwise a server started beside
  another would come up half listening.
- **A warning when listening on loopback with no trusted proxies.** Behind a proxy on its own
  machine every client would share the proxy's address.
- **Connection limits and the header timeout.** Connections that send nothing or trickle their
  headers are let go.
- **The outbox is bounded in bytes and a lagging socket is closed.** A client that stops reading
  cannot pile up everyone else's frames on the server. Bytes, because frames differ in size.
- **Mutes and deafens are never rate limited; only lifting counts.** Nothing, a flood of others'
  frames included, can keep someone's microphone open after they closed it, and since each mute must
  be undone before it repeats, limiting the lifts bounds both.
  ([The voice server process](voice-server.md#muting-is-never-limited))
- **Frame counters are in process.** Every client of a call talks to this one server.
- **Seats are counted on the voice server, not the API server.** The API server's record follows
  reports; the voice server holds the calls, so its count is exact.
- **Connections are bounded per participant, per user, and per call, and what one participant
  receives whatever the call's size.** Each holds memory and a share of its media thread's time.
  ([The voice server process](voice-server.md#seats-and-connections))
- **A stopped media thread stops the whole server.** Leaving that thread's rooms would look alive
  with no media; stopping sends everyone to rejoin and lets the supervisor restart the process.
  ([Signalling](signalling.md#a-media-thread-stopping))
- **A panic inside one connection's str0m calls drops that connection alone.** str0m panics only on
  its own broken invariants, never on input; catching it there costs one participant a rejoin rather
  than every call on the thread. A panic anywhere else may have left the thread's own state broken,
  so it stops the server.

## Media threads and str0m

- **str0m, a Rust library, rather than mediasoup, a C++ worker.** In the evaluation they forwarded
  at parity, about 2.5 µs of CPU per packet delivered. str0m runs inside the voice server's own
  threads, so a fault in one connection can be caught there, the build needs no C++ toolchain,
  Meson, or Python, and the server links no OpenSSL (mediasoup's worker compiles in the OpenSSL it
  bundles, or the system's through a fork). Its SDP is what browsers and the phones' libwebrtc speak
  natively, where mediasoup needs a client library of its own on every platform. The media threads
  measure audio at about 2.5 µs a packet and video at about 4.8 µs, a few percent of a core at the
  rates calls send, left there.
- **One UDP and one TCP port per media thread, on every media address.** An operator opens as many
  ports as threads, not a range sized to people, and a firewall rule never grows with load. Every
  connection on a thread shares its sockets: datagrams are told apart by the address ICE paired, and
  a first STUN request by the username fragment it names. TCP, framed per RFC 4571, is there for
  clients whose networks block UDP. ([The voice server process](voice-server.md#media-threads))
- **Each UDP socket asks for 4 MiB of buffer each way, and the server warns when it gets less.**
  Every connection on a thread shares the socket, and the evaluation dropped DTLS handshakes at
  about 1,200 connections on the kernel's default 208 KiB.
- **A call lives on one media thread.** Forwarding never crosses threads, so a packet's path takes
  no lock.
- **Resends may be half of what a stream sends.** Over a lossy, slow link a receiver asks again for
  packets whose resends are still on their way, and str0m's default cap (resends at most 15% of a
  stream's bytes) dropped most of what such a link needed: through a relay losing 5% each way with
  50 ms of delay, the picture froze 38 times in 30 seconds, and 11 times with the cap at half, where
  mediasoup froze 15 to 28. Half still bounds a receiver that asks for everything to making the
  server send twice the media.
- **Audio lost on its way up is not recovered.** mediasoup NACKs a browser's Opus, which a browser
  answers by sending the packet again; str0m NACKs only a stream with RTX, which browsers never use
  for audio. Through a relay losing 5% toward the server alone, a listener lost 4% of the audio
  where mediasoup lost 0.07%. Kept for now, as a known shortfall against mediasoup; closing it means
  str0m NACKing such streams (a change to str0m), or RED (RFC 2198), which also covers the way down
  at the cost of about twice each voice's bitrate.

## Signalling

- **The server is the only side that offers, and a connection has at most one offer outstanding.**
  Whatever the client wants is a request the server turns into an offer, and changes made while one
  waits go into the next. Neither side ever handles an offer collision, which keeps the phones'
  native engines and the browsers alike simple. ([Signalling](signalling.md#offers-and-answers))
- **Each offer names what every one of its sections carries, in the same frame.** A client places
  each track as it arrives, with nothing to wait for or reconcile.
- **One connection per participant carries both ways.** One ICE and DTLS per person, one set of
  candidates, one thing to restart when a phone changes network. Two connections, one each way,
  would let what is sent and what is received renegotiate apart, at twice the handshakes, restarts,
  and connections on the media thread; with receiving bounded, renegotiations are rare and small, so
  one queue of them costs little. The game capture helper is a process of its own, and so a
  connection of its own. ([Signalling](signalling.md#connections))
- **The server offers an ICE restart when a connection's ICE goes disconnected.** A phone moving
  between Wi-Fi and cellular recovers without rejoining.
- **A socket that closes without `leave` keeps its participant for twenty seconds, media flowing.**
  A phone's web view can freeze while its native engine keeps the call, and a socket drops on a
  network change while the media path recovers; the client takes its place back with `resume` and
  nothing is renegotiated. Twenty seconds covers both, and keeps someone truly gone shown in the
  call no longer than that. The secret is single use and made anew with each `ready`, so a socket
  that saw one cannot take the place back later. ([Signalling](signalling.md#resuming))

## Receiving

- **What a client receives is bounded whatever the call's size.** An SDP section costs about 900
  bytes; a section per remote source would make a 500-person call's offer about 450 KB, past the
  signalling message limit, and send it to everyone on every join.
- **Audio slots, filled by whoever is loudest.** A client receives a fixed number of audio sections,
  so joins and leaves past that renegotiate nothing. A phone decodes at most sixteen voices, and
  past sixteen speaking at once nobody follows anyway. ([Signalling](signalling.md#audio-slots))
- **A connection gains a slot per other microphone up to sixteen, and never loses one.** Every
  section costs its connection on each poll whether or not it carries anything, so a call of six
  holds five; a call renegotiates only while it grows, and a leave never takes a slot away.
- **`slots` says whom each slot carries, and clients attribute by it.** libwebrtc's C++ API has
  contributing sources, but neither its Android nor its iOS layer exposes them. The packets carry
  the speaker's SSRC as their CSRC all the same, for clients that read it.
- **A slot changes occupant by offer, into the occupant's stream.** Their voice must play in sync
  with their camera (see [Sync and silencing](#sync-and-silencing)), which a browser does only for
  tracks of one stream; the cost is a renegotiation per change of speaker, which happens only once
  more microphones are in the call than slots. In a call of forty
  (`voice_media/tests/slot_churn.rs`), three speaking at once with a new one every two seconds cost
  each listener 22 offers a minute of 16 KB each, about 48 kbit/s of signalling; twenty speaking at
  once, four changing each second, cost 66 a minute, the most one outstanding offer at a time
  allows, about 140 kbit/s. An offer deflates to about a ninth of its size, so signalling is
  compressed (below).
- **Signalling is compressed, by the WebSocket rather than in the frames.** permessage-deflate is
  negotiated by every browser and web view on its own, so no client and no frame changes, and the
  realistic call above costs a listener about 5 kbit/s. Neither axum's WebSocket nor tungstenite has
  it, so the voice server's signalling uses `yawc`. No context is kept between messages: each offer
  compresses about as well alone (it repeats itself section to section), and a socket then holds a
  few kilobytes rather than the few hundred a deflate state of its own would take, across thousands
  of sockets.
- **A muted microphone gives up its slot at once.** It sends nothing, so holding a slot would keep
  someone who speaks out of the slots.
- **Video only as watched, up to 25.** A client says what it shows, and the server sends only that,
  so a phone never receives video it would discard, whose decoding costs battery.
  ([Signalling](signalling.md#video-by-watching))
- **Bandwidth estimation is off on the server.** With one encoding per source, nothing would act on
  an estimate, and in the evaluation it cost 20 to 40% more CPU. Senders' own estimates work from
  the transport-wide feedback the server returns.

## Sync and silencing

Sync is a correctness requirement: a person's voice with their camera, and a share's picture with
its sound, play in step wherever the media carries what synchronizing them needs.

- **A browser synchronizes only tracks of one stream, so each section names its stream.** A person's
  slot and camera are in `{user}-camera`, a share's picture and sound in `{user}-share`. The two
  pairs are apart because a game capture's picture and sound come from the desktop helper on a
  connection and clock of their own, not with the camera.
  ([Signalling](signalling.md#offers-and-answers))
- **A section moved into another stream is sent on new SSRCs.** libwebrtc (Chromium, and the engine
  Firefox and the phones use) puts a receive stream into a sync group when it makes it for an SSRC,
  and makes none for an SSRC it already has: in Chromium, a track whose `msid` alone changed never
  synchronized, and the same change on a new SSRC synchronized within seconds. str0m had no way to
  move a section into another stream, so the voice server builds on a fork of it whose
  `SdpApi::set_stream_id` does both, pinned in the workspace's `Cargo.toml` until str0m releases it.
- **Sections no longer wanted are kept idle and taken over, never stopped and added.** str0m does
  not recycle a stopped m-line, so every stop and add would leave about a kilobyte in every later
  offer for the rest of the call. An idle section keeps its numbering: a receiver's SRTP drops
  numbers below those it has had on an SSRC as replays, and a section taken over in its own stream
  that numbered from one again went unseen in Chromium and Firefox for as long as the numbering took
  to catch up, tens of seconds into a call.
- **Each packet is sent with its sender's capture time, on the server's clock.** The server's sender
  reports give each stream's RTP time at a wallclock, and a receiver synchronizes two streams by
  those. Taking the time a packet arrived would put the network's and the sender's encoders'
  differing delays between a person's audio and video; taking the sender's own reports keeps the
  timing the sender captured with.
- **The sender's clock is paired with the soonest arrival seen, not the first.** str0m's reports
  carry a stream forward from its last packet's wallclock to the report's, and a wallclock not yet
  come gives the report no RTP time at all. Pairing on the first report's arrival placed every
  quicker packet in the future, and Chromium then either made no estimate or settled tens of seconds
  off.
- **A new slot occupant's audio is held until the answer, up to a second.** The slot carries the old
  speaker until the client knows it moved, and dropping what the new one says meanwhile would cut
  the start of what they say; a second covers an answer's round trip on the worst networks a call
  works over.
- **The desktop helper stamps every track it sends on one capture clock.** Its game picture and
  sound are encoded on threads of their own; stamping each frame with the instant it is sent would
  put each encoder's delay between them.
- **The call check measures sync rather than assuming it.** It holds a stream's picture 400 ms back
  at the receiver (`jitterBufferTarget`) and requires the browser to bring the sound within 45 ms,
  read from both tracks' `estimatedPlayoutTimestamp`, which a browser reports only for tracks it
  synchronizes: unsynchronized tracks leave no estimate, or one 400 ms apart. 45 ms of sound ahead
  is where ITU-R BT.1359 puts the edge of what people notice. A browser makes its first estimate
  once sender reports have come, a few seconds in, and closes the gap by about 30 ms a second
  (Chromium and Firefox alike), so the check allows 45 seconds.
- **Someone a listener plays at nothing takes none of their slots.** Slots go to the loudest, and
  without the listener's volumes the server would fill one with a person the listener never hears,
  leaving a voice they would hear out. The list goes to the server rather than each volume, since
  the server needs only whether a voice is heard, and the client's levels between stay its own.

## Media

- **H.264 is offered after VP8.** A browser answering an offer follows its order, and Chromium's
  H.264 encoder is the software OpenH264, which drops frames on screen content. H.264 is there for
  the desktop shell's game capture, whose connection offers it alone.
  ([Signalling](signalling.md#codecs))
- **A share's sound is stereo, without DTX, at 128 kbit/s; a microphone keeps DTX.** A browser
  decodes and sends Opus as mono without `stereo` and `sprop-stereo`, and DTX cuts the quiet
  stretches of music and games. str0m sets format parameters per connection, so the server writes
  them into its offers per section; it only parses what clients answer, so editing its offers' text
  costs it nothing.
- **Video starts at the browser's own bitrate; the offers set no `x-google-start-bitrate`.** A share
  is soft for its first seconds while the sender's bandwidth estimate climbs. Offered 10 Mbit/s, a
  browser starting its camera and screen together encoded more than it could send, and its video
  reached the server up to 4 s after capture in the first seconds. Receivers then took over a minute
  to settle back into lip sync, with the sound up to 150 ms behind the picture, and the call check's
  sync phase failed; at the browser's own rate, video arrives within 10 ms of capture from the start
  and the check passes. A start above the default needs both measured again: how far behind capture
  video reaches the server, and the sync phase.
- **A video section waits for a keyframe, and keyframe requests reach a source at most every 300
  ms.** A decoder can start only on one; however many receivers start or resume at once, the sender
  is asked once.
- **Synthetic H.264 leads each keyframe with an SPS and PPS.** The calibration, the bench, and the
  tests then send what an encoder sends.
- **A room closes the moment its last participant leaves, and newcomers wait for its end to be
  reported.** The API server never hears of a new session before the old one's end, and nobody joins
  a room about to go. ([Signalling](signalling.md#rooms))
- **Every frame acts through the participant its socket made, and a leaving participant hangs its
  socket up.** A socket never acts for its successor or in a later call, and nothing is left open on
  a place it no longer holds.
