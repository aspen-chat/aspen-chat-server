# Voice design notes

Why the voice pieces are shaped as they are. The how is in the [voice pages](index.md).

## Registry

- **`remove` refuses a server that holds calls.** Ending them takes a server's context. ([Servers](servers.md))
- **`add` also updates a server of the same name.** A deployment's scripts may then run it on every deploy. ([Servers](servers.md))

## Join tokens

- **Tokens are signed with an Ed25519 key, and voice servers hold only the public half.** A voice server can check tokens but never make one. ([Join tokens](join-tokens.md))
- **The public half is fetched over NATS from the API servers' queue group.** Only the API servers may publish to a voice server's inbox, so the answer is theirs. Asking again for an unknown key is capped at once every five seconds. ([Join tokens](join-tokens.md#how-a-voice-server-gets-the-public-half))
- **The key request names the voice server's id and the answer says whether it is registered.** A voice server given the wrong id stops at startup rather than reporting to no one. The id is only asked about, never believed. ([Join tokens](join-tokens.md#checking-the-voice-servers-id))
- **Registration look-ups are capped at `REGISTRATION_LOOKUPS` (two) per API server.** A voice server asking over and over holds at most that many database connections.
- **Key answers go only to the asking server's inbox.** Any other reply subject is a sign of a compromised voice server. ([Join tokens](join-tokens.md#how-a-voice-server-gets-the-public-half))
- **Shared-secret tokens are still accepted under `token_secret`.** A deployment can then upgrade from API servers that made them without breaking calls. ([Join tokens](join-tokens.md#shared-secret-tokens))
- **A token admits one connection to each server it names, tracked by nonce.** Someone removed cannot come back on the token they joined with. ([Join tokens](join-tokens.md#one-connection-per-token))

## Producers by source

- **The voice server checks a producer's kind against its source.** What may be sent and what a mute pauses are decided by source. ([Rechecks](rechecks.md#what-the-voice-server-enforces-on-each-producer))
- **Grants are checked again under the room's lock as the producer is added.** Making a producer awaits mediasoup, and a `Grant` may land meanwhile; that `Grant` finds nothing to close, so the second check closes the producer instead. ([Rechecks](rechecks.md))
- **One producer per source.** No client can multiply what everyone else in the call receives. ([The voice server process](voice-server.md#seats-and-transports))

## Rechecks

- **Rechecks run after the work commits, from `rechecks_of`, which matches every event with no wildcard.** No call site needs to remember them, and a new event cannot be published until its effect on calls is decided. ([Rechecks](rechecks.md))
- **Rechecks run on their own tasks, at most `RECHECKS_AT_ONCE` (16) at a time per server, and a recheck of every call runs as a job.** Turning file transfers on or off reaches every call on the deployment, so `recheckAllCalls` takes a batch of seats at a time. ([Rechecks](rechecks.md#when-a-recheck-runs))
- **A join's report rechecks the joiner, and is checked against live sign-ins.** A token issued before a change, or before its sign-in ended, and used after it, is brought in line once the join is recorded. ([Rechecks](rechecks.md#ended-sign-ins))
- **A recheck always sends `Mute` with the mute as it stands, and the voice server ignores no-ops.** A token issued before a change is brought in line once its join is recorded. ([Moderation](moderation.md#how-it-reaches-the-call))

## Server selection

- **Candidates are chosen at random for now**, at most `candidate_limit`; the client picks among them by measured latency. ([Choosing a server](server-selection.md))
- **A server silent for `offer_silence_seconds` is not offered.** It is probably down, and offering it would cost clients failed attempts and count against it.

### Failure reports

- **A report counts only from someone offered the server who has not joined a call there within the window.** No one can suspend a server they were never sent to or reached. ([Choosing a server](server-selection.md#which-reports-count))
- **Bots and foreign users' reports do not count.** A bot's owner may have many bots; a foreign user is vouched for by another deployment.
- **A Valkey outage is read as having joined.** Reports then count for nothing rather than for anyone.
- **A server is never suspended when no other would be left taking calls, decided with every row locked.** Throwaway accounts can at worst suspend every server but one.
- **A suspension ends on its own after the window.** By then the server's failures are all older than the window, so it is offered again without an operator.

## Reports

- **Reports are partitioned by channel into lanes, each with one shared durable consumer handing out one report at a time.** The reports about one channel are applied once and in order by whichever API server takes them, while lanes are applied side by side, so a busy channel holds up only its lane. ([Voice reports](voice-reports.md#lanes))
- **Speaking changes have lanes of their own.** They come many times faster than anything else, and never hold up joins and leaves.
- **Report application is capped at half the database pool.** Requests always have the other half.
- **A report's source is the server its subject names, never what it says.** With per-server NATS users, a voice server taken over can misreport its own calls and no one else's. ([Voice reports](voice-reports.md#where-a-report-came-from))
- **A new call or participant needs an offer note.** A voice server taken over cannot make up a call, or someone in one, anywhere it was not sent, to ring a DM or show a call that is not there.
- **A Valkey outage believes the report.** Calls go on through it.
- **Applying a report is idempotent.** A report whose acknowledgement was lost is delivered again.
- **Reply subjects can still be aimed elsewhere.** NATS checks a reply subject against no one's permissions, so a voice server taken over can have the JetStream API publish on the API servers' subjects; none of that is believed as a report, a command, or an event. Only a NATS account of the voice servers' own would keep replies within it. ([Voice reports](voice-reports.md#what-a-voice-server-taken-over-can-still-cause))
- **Where each session is, is remembered for ten seconds for speaking changes.** A session's server and channel never change, so a lively call reads the database once per call every ten seconds rather than once per change. ([Voice reports](voice-reports.md#speaking-changes))
- **A load report from an unregistered id updates nothing.** It is noted for the dashboard instead.
- **`load` records when the stream took the report.** A report that waited in the stream does not make a dead server look alive.

## Snapshots

- **Voice servers send snapshots every minute and on reconnecting.** Reports can be lost while NATS is unreachable, when NATS restarts, or by being dropped. ([Snapshots](snapshots.md))
- **A snapshot's parts are queued together, each after every earlier report about its calls.** Each part then describes things as of its place in the order.
- **Each repair logs a warning.** One means a report was lost.
- **A server's first snapshot waits a full interval.** The calls it held before a restart are carried on by their people rejoining rather than ended first.

## Sessions

- **`session_silence_seconds` is long (a day).** A dead voice server never leaves phantom calls, while a brief outage of the report link does not end anyone's call. ([Sessions](sessions.md#silence-and-the-reaper))
- **`idle_session_seconds` ends calls that never held two people.** A forgotten client cannot hold a voice server slot.
- **Ending a call locks its session row.** The reaper and a report, ending one call at once, record the end once.
- **A duplicate room on another live server is closed, not recorded.** Join tokens name up to ten servers, so two rooms can open at once; the recorded call wins and the other's people are sent to it. ([Sessions](sessions.md#duplicate-rooms))
- **Rejoining waits a random delay of at most a second.** The participants of a lost call do not all hit the API server in the same instant.

## Moderation

- **The participant endpoints answer `202 Accepted`.** The voice server applies the command and its report changes the record, so the response is not the result. ([Moderation](moderation.md))
- **A server mute is the community's and outlives leaving.** Leaving and coming back does not lift it.

## The voice server

- **The announced address may never be loopback.** Firefox does not pair its own host candidates with a loopback peer, so signalling would succeed and media would never flow. ([The voice server process](voice-server.md#the-announced-address))
- **A warning when listening on loopback with no trusted proxies.** Behind a proxy on its own machine every client would share the proxy's address.
- **Connection limits and the header timeout.** Connections that send nothing or trickle their headers are let go.
- **The outbox is bounded in bytes and a lagging socket is closed.** A client that stops reading cannot pile up everyone else's frames on the server. Bytes, because frames differ in size.
- **Mutes and deafens are never rate limited; only lifting counts.** Nothing, a flood of others' frames included, can keep someone's microphone open after they closed it, and since each mute must be undone before it repeats, limiting the lifts bounds both. ([The voice server process](voice-server.md#muting-is-never-limited))
- **Frame counters are in process.** Every client of a call talks to this one server.
- **Seats are counted on the voice server, not the API server.** The API server's record follows reports; the voice server holds the calls, so its count is exact.
- **Transports are bounded per participant, per user, and per call.** Every transport takes ports from the media range.
- **A dead worker stops the whole server.** Leaving that worker's rooms would look alive with no media; stopping sends everyone to rejoin and lets the supervisor restart the process. ([Signalling](signalling.md#a-worker-dying))

## Media

- **H.264 is listed after VP8.** A browser producing video takes the router's first codec it can send, and Chromium's H.264 encoder is the software OpenH264, which drops frames on screen content. H.264 is there for the desktop shell's game capture. ([Signalling](signalling.md#codecs))
- **Game audio declares `sprop-stereo`.** Consumers inherit the producer's codec parameters, and a browser decodes Opus as mono without it.
- **`INITIAL_OUTGOING_BITRATE` is 10 Mbps, above mediasoup's 600 kbps default.** A share reaches its viewers sharp from its first seconds; each receiver's bandwidth estimate brings it down where it must.
- **Synthetic H.264 leads each keyframe with an SPS and PPS.** mediasoup recognises a keyframe by its SPS, and a consumer forwards nothing until it has seen one.
- **A room closes the moment its last participant leaves, and newcomers wait for its end to be reported.** The API server never hears of a new session before the old one's end, and nobody joins a room about to go. ([Signalling](signalling.md#rooms))
- **Every frame acts through the participant its socket made, and a leaving participant hangs its socket up.** A socket never acts for its successor or in a later call, and nothing is left open on a place it no longer holds.
