# Signalling and media

Signalling runs over the voice server's `GET /ws` socket. Frames are `voice_protocol::signal`. Media
runs over WebRTC peer connections the server holds on its media threads (`voice_media`; see
[The voice server process](voice-server.md#media-threads)).

## Where it lives

| Piece                         | Code                                                                                                                                               |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Frames                        | `voice_protocol::signal`                                                                                                                           |
| The socket                    | `voice_server/src/signalling.rs`                                                                                                                   |
| Rooms: who may do what        | `voice_server/src/rooms.rs`                                                                                                                        |
| Connections, offers, sections | `voice_media/src/connection.rs`                                                                                                                    |
| Forwarding, slots, levels     | `voice_media/src/thread.rs`, `levels.rs`, `keyframe.rs`                                                                                            |
| Per-section format parameters | `voice_media/src/sdp.rs`                                                                                                                           |
| A media thread stopping       | `Stopped::MediaThread` in `voice_server/src/main.rs`                                                                                               |
| A socket's place              | `Seat`                                                                                                                                             |
| Client                        | `VoiceCall` in `client/packages/protocol/src/voice.ts`                                                                                             |
| Test server                   | `voice_protocol/examples/fake_voice_server.rs`, which sends any report or command by hand. The API server's side is tested this way without media. |

Screen sharing and game capture in the client are in
[Screen sharing and game capture](../../../client/docs/architecture/screen-sharing-and-game-capture/index.md).

## Joining

1. The client sends `identify` with the join token.
2. The server answers `ready`: the session, who is in the call and what each sends, the most audio
   slots its offers carry (`audioSlots`), the most videos it may watch (`maxWatchedVideo`), a secret
   to resume with, and the call's file offers and transfers.
3. The media thread offers the client's call connection (`offer`) with one audio slot, a connection
   needing a section to connect at all. The offer follows `ready`, being asked for under the room's
   lock. When others are in the call, `slots` then says whom each slot carries, those still to be
   offered among them.
4. The client answers (`answer`, with the offer's revision), and asks to send its microphone
   (`send {source: "microphone"}`).
5. Once the first offer is answered, the next adds a slot for each other microphone in the call, up
   to `audioSlots`, and a section for the microphone. The client answers with its track on that
   section.
6. Once the microphone's first packet arrives, the server says `sending` to the client and `sources`
   to everyone.

## Offers and answers

The server is the only side that offers. Whatever the client wants changed is a request frame
(`send`, `stopSending`, `watch`, `restartIce`); the media thread makes the change and offers it.

- Every `offer` names its connection (`call` or `capture`), a revision, the SDP, and what every
  section carries (`sections`): a source the client sends, an audio slot, another participant's
  video, or a share's sound. A section is named with the offer that adds it, never after.
- A connection never has two offers outstanding. Changes made while one waits are gathered into the
  next, offered as soon as the answer is applied. So neither side handles a collision.
- An answer is to the revision it names. An answer to a revision the connection no longer waits on
  (given twice, or late) is let be, and the outstanding offer still stands.
- An offer unanswered for `ANSWER_TIMEOUT` (ten seconds), or an answer that cannot be applied, fails
  the connection.
- Nothing is sent on a section before the answer to the offer that added it.
- When ICE on a connection goes disconnected (a client that moved network), the server offers an ICE
  restart. A client asks for one with `restartIce`, again while it stays lost. Either way the next
  offer carries new ICE credentials, and the connection is found by the new username fragment from
  then on: a browser checks a new generation from ports of its own, which no route knows yet.

### A connection failing

| Failure                                                                       | `Failure`        |
| ----------------------------------------------------------------------------- | ---------------- |
| An offer unanswered for ten seconds                                           | `AnswerTimedOut` |
| An answer that cannot be applied                                              | `BadAnswer`      |
| ICE or DTLS ended it                                                          | `Disconnected`   |
| ICE and DTLS not done within `CONNECT_TIMEOUT` (thirty seconds) of its making | `NeverConnected` |
| str0m panicked inside one of this connection's calls (caught)                 | `Internal`       |

- A call connection that fails takes its participant out of the call with `kicked`
  `connectionFailed`, and the client rejoins.
- A capture connection that fails stops the sources the helper sends, tells the others (`sources`),
  and tells the client with a non-fatal `error` naming `send` as `refused`, so it stops whatever
  still waited to be sent on it.

## Connections

| Connection | Held by                                                          | Carries                                                                             |
| ---------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `call`     | Every participant, from joining to leaving                       | Everything they send from the browser, and everything they receive                  |
| `capture`  | The desktop shell's game capture helper, while it sends for them | The sources asked for with `external: true`: a game's picture (H.264) and its sound |

- One call connection per participant carries both ways: one ICE and DTLS, one set of candidates,
  one thing to restart on a network change.
- The capture connection is made with the first `external` source and goes with the last. It is
  negotiated through the client like any other: the client hands each `capture` offer to the helper
  and returns its answer.
- Every connection is ICE lite on the server's side, with host candidates of every media address:
  UDP, and passive TCP for clients UDP cannot reach.

## Sending

| Source        | Kind  | Sent from                                                                                                  |
| ------------- | ----- | ---------------------------------------------------------------------------------------------------------- |
| `microphone`  | audio | the call connection                                                                                        |
| `camera`      | video | the call connection                                                                                        |
| `screen`      | video | the call connection, or the capture connection (a game, `external`)                                        |
| `screenAudio` | audio | the call connection (sound the browser captured with a screen), or the capture connection (a game's audio) |

1. `send {source, external?}` asks for a section. The server checks the grant, refuses a source
   already sent (one of each per participant), and offers a receiving section for it on the call
   connection, or on the capture connection when `external`.
2. The client answers with its track on that section.
3. The source's first packet makes it live: the server checks the grant again, says `sending` to the
   client, and tells everyone what the participant now sends (`sources`). The others are told of a
   source only once its media arrives.

- `stopSending` ends one: its section goes from the next offer, and the others are told when it was
  live.
- Switching a device changes the track on the same section, with no signalling.
- A refused `send` is a non-fatal `error` naming `send` as `refused`.
- The grants each source needs are in
  [Rechecks and grants](rechecks.md#what-the-voice-server-enforces-on-each-source).

## Receiving

What a client receives is bounded whatever the size of the call, so an offer's size is too.

### Audio slots

- A call connection starts with one audio slot and gains one for each further other microphone in
  the call, up to `AUDIO_SLOTS` (sixteen, `voice_server/src/rooms.rs`). It never has fewer than it
  had: a call renegotiates only while it grows towards the limit.
- While everyone else's microphone fits, each has a slot, and slots move only as people come and go.
- Past that, at the end of each 300 ms level interval, the slots go to those speaking now, then the
  most recently loud, then who joined first. A muted microphone, which sends nothing, ranks last and
  gives up its slot at once. An occupant stays in its slot while it is among those chosen, so a slot
  changes speaker only to make room.
- `slots` tells the client whom each slot now carries, those that changed, and is sent before the
  offer that adds a new slot's section. The client plays a slot at the volume it gives that person.
- A slot keeps one SSRC throughout. Its sequence numbers and timestamps are rebased at a new
  speaker's first packet, so they run on unbroken across a switch, and each packet carries the
  speaker's SSRC as its CSRC for clients that read contributing sources.

### Video by watching

- `watch {video}` names the video the client shows, replacing what it watched before, at most
  `MAX_WATCHED_VIDEO` (25, given in `ready` as `maxWatchedVideo`); those past it are left out.
- The server offers a section for each watched source that is being sent, and takes away those no
  longer named. A source watched before it is sent gets its section once it is.
- A watched screen whose sender also sends `screenAudio` brings a `shareAudio` section beside it,
  except to the sharer themself: a client watches its own game share for a preview, and is not sent
  its sound back.
- A video section starts on a keyframe. Until one arrives the server asks the source for one by PLI,
  at most once per source every 300 ms however many receivers wait. A receiver's own keyframe
  request goes on to the source the same way.
- `setReceiving {mid, receiving}` stops or resumes one received section with no renegotiation: video
  off screen, someone muted for this client alone. A resumed video section waits for a keyframe
  again.

## Forwarding

- Packets are forwarded as they arrive (str0m's RTP mode), never decoded. Each section a client
  receives has its own SSRC, with sequence numbers and timestamps rebased whenever the source it
  carries starts over (a new slot occupant, a source paused and resumed, a section resumed).
- Payload types are mapped to the receiving connection's own.
- NACK and retransmission are per leg, video only. A retransmission from before a source began on a
  section is dropped. A sending stream resends from its last three seconds, up to 1,024 packets, and
  resends may be up to half of what it sends in a second (`RESEND_SHARE_CAP` in
  `voice_media/src/connection.rs`) before those still queued are dropped.
- Audio lost on its way to the server stays lost: str0m NACKs only a stream that has RTX, which
  browsers do not send for Opus. Opus's in-band FEC and the receiver's concealment cover it.
- The audio level and voice activity extensions are carried on.
- Bandwidth estimation is off on the server. Each sender's own estimate works from the
  transport-wide feedback the server returns.

## Codecs

Every connection offers:

- Opus for audio (10 ms minimum packet time, in-band FEC, DTX),
- on a call connection, VP8 then H.264 constrained baseline for video, each with RTX,
- on a capture connection, H.264 alone, which is what the helper encodes.

The server edits format parameters into its offers per section, where str0m sets them per connection
(`voice_media/src/sdp.rs`):

| Section                      | Parameters                                                           |
| ---------------------------- | -------------------------------------------------------------------- |
| Sending `screenAudio`        | `stereo=1`, `sprop-stereo=1`, `usedtx=0`, `maxaveragebitrate=128000` |
| Sending `screen` or `camera` | `x-google-start-bitrate=10000` on VP8 and H.264                      |
| Receiving a share's sound    | `stereo=1`, `sprop-stereo=1`                                         |

VP8 comes first, so a browser sending video sends VP8. See [design notes](design-notes.md#media).

## Mute and deafen

- Mute pauses the microphone at the server: nothing of it is forwarded, its levels stop, and the
  others are told it is paused (`sources`). A microphone asked for while muted starts paused.
- Deafen stops the audio slots and the shares' sound sent to the participant. A shared screen stays
  visible to a deafened participant.
- Mute never touches `screen` or `screenAudio`.
- Both are reported to the API server.
- Someone muted while speaking stops speaking at once (`speaking` to the call, and the report),
  rather than at the next interval. Nobody else's speaking changes.

### A moderator's mute

A moderator's mute (`VoiceCommand::Mute`; see [Moderation](moderation.md)) is a flag of its own
beside the participant's own mute.

- While it stands, their microphone stays paused, and a new one starts paused.
- A `setState` from their client changes only their own mute.
- The mute everyone is told of is either one: in `participantState`, `ready`, the reports, and so
  `VoiceParticipant.muted`.
- A moderator's unmute lifts only the moderator's. Someone who muted themself stays muted.

## Speaking

Each microphone and share sound carries its RFC 6464 audio level, which the server's offers ask for.
Over each 300 ms interval a source's levels are averaged; above -50 dBov it is speaking
(`voice_media/src/levels.rs`). A participant speaks while any of their sources does. A paused
microphone never speaks. The same levels fill the audio slots.

## State reports

- The `participantState` report carries `sharing_screen`. It is sent when a screen goes live or
  stops, as well as with mute and deafen changes, and after the join of a participant who replaced
  another socket's or joined muted by a moderator.
- `VoiceParticipant.sharingScreen` follows it, so a client can mark who is sharing without being in
  the call.

## Resuming

A socket that closes without `leave` leaves its participant in the call for `RESUME_GRACE_SECONDS`
(twenty), media flowing.

1. The client opens a new socket and sends `resume {secret}` in place of `identify`, with the secret
   its last `ready` gave.
2. The server takes the participant back: the new socket's outbox replaces the old (any old socket
   still open is hung up), and it answers `ready` with a new secret. A secret is single use.
3. It restates each connection: its outstanding `offer` again, or, where none is outstanding, its
   `sections`.

- What happened while the socket was away (a kick, a grant change, a moderator's mute) applied to
  the participant at once, and is told in `ready`.
- A participant whose grace runs out leaves as one who left.
- `resume` is limited by address before the secret is looked up, so secrets cannot be guessed at
  speed. A secret that names no one waiting is a fatal `error`, and the client joins again.

## Rooms

A call is a room: on one media thread, created with the first participant and closed with the last.
Each step is reported. Calls are given to media threads round robin.

- Two joins that start a channel's call at once share the room the first of them made.
- A room closes the moment its last participant leaves. Whoever arrives while it is still being torn
  down waits until its end is reported, then starts the channel's next call. The API server never
  hears of the new session before the old one's end, and nobody is let into a room that is about to
  go.
- Offers and transfers are made under the room's lock with their participants checked present. One
  is either made before its people leave, and ended as they do, or refused.

### A media thread stopping

A media thread that stops stops the whole server (`Stopped::MediaThread`), as stopping it would:

1. Everyone is told the server is stopping, so their clients rejoin.
2. The process exits with an error for its supervisor to start it again.

A panic inside one connection's str0m calls is caught instead, and fails that connection alone
(`Internal`).

## Sockets and participants

- A user who joins again from another socket replaces their participant.
- What the replaced participant had in the call goes as it does when one leaves: its connections
  close, everyone is told it sends nothing (`sources`), its offers are withdrawn and its transfers
  end (both as `left`), and it stops speaking. No `participantLeft` is sent or reported, since the
  user is still in the call.
- The replacement's join is reported, and then its state (`participantState`): the API server keeps
  the record of someone already in the call as it is, and that record describes the socket replaced.
- A socket leaving takes out only the participant it made. The replaced socket closing never takes
  out its successor.
- Every frame from a socket acts through the participant it made or resumed (`Seat`: the channel,
  the user, and the connection). It is refused once that participant is gone. A socket never acts
  for its successor, nor in a later call in the same channel.
- A participant leaving by any path (removed by a moderator or a recheck, replaced, the server
  stopping, a failed connection, a resume grace run out) hangs its socket up, after the `kicked`
  frame saying why. Nothing is left open on a place in the call it no longer holds.

## Frames

### From the client

| Frame                                                                      | Meaning                                                                          |
| -------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `identify {token}`                                                         | First frame: the join token from the API server                                  |
| `resume {secret}`                                                          | First frame in place of `identify`: takes back a participant whose socket closed |
| `answer {connection, revision, sdp}`                                       | The answer to that revision's offer                                              |
| `send {source, external?}`                                                 | Asks to send a source; `external` sends it from the capture connection           |
| `stopSending {source}`                                                     | Ends one                                                                         |
| `watch {video: [{user, source}]}`                                          | The video the client shows, replacing what it watched                            |
| `setReceiving {mid, receiving}`                                            | Stops or resumes one received section, with no renegotiation                     |
| `restartIce {connection}`                                                  | Asks for an ICE restart in the next offer                                        |
| `setState {muted, deafened}`                                               | The participant's own mute and deafen                                            |
| `offerFile`, `withdrawFile`, `acceptFile`, `transferSignal`, `endTransfer` | See [File transfers](../file-transfers.md)                                       |
| `leave`                                                                    | Leaves now, without waiting out the resume grace                                 |

### From the server

| Frame                                                                                                        | Meaning                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| `ready {session, user, participants, audioSlots, maxWatchedVideo, resume, offers, links, transfers}`         | The reply to `identify` and `resume`; each participant with what they send (`sources`)                                         |
| `offer {connection, revision, sdp, sections}`                                                                | An offer to answer, and what each of its sections carries                                                                      |
| `sections {connection, sections}`                                                                            | What each section carries, after a `resume` where no offer is outstanding                                                      |
| `sending {source}`                                                                                           | A source the client asked to send carries media at the server, and is sent on                                                  |
| `sources {user, sources}`                                                                                    | What a participant sends now, each source with whether it is paused                                                            |
| `slots {assignments}`                                                                                        | Whom the client's audio slots carry from now on, those that changed                                                            |
| `participantJoined`, `participantLeft`, `participantState`, `speaking`                                       | Who is in the call, their mute and deafen, and who speaks                                                                      |
| `kicked {reason}`                                                                                            | The server closed the client's place: `kicked`, `replaced`, `serverStopping`, `accessLost`, `connectionFailed`, or `signedOut` |
| `grantsChanged {grants}`                                                                                     | What the client may now do; see [Rechecks and grants](rechecks.md#grants-change)                                               |
| `fileOffered`, `fileWithdrawn`, `transferStarting`, `transferSignal`, `transferEnded`, `transferLinkChanged` | See [File transfers](../file-transfers.md)                                                                                     |
| `error {detail, fatal, retryAfterSeconds?, refused?}`                                                        | A request not honoured; the socket stays open unless `fatal`                                                                   |

Each frame type's rate limits are in `voice_server/src/limits.toml` (see
[Frame rate limits](voice-server.md#frame-rate-limits)).
