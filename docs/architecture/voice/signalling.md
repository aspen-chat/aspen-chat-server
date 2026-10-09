# Signalling and media

Signalling runs over the voice server's `GET /ws` socket. Frames are `voice_protocol::signal`.

## Where it lives

| Piece | Code |
|---|---|
| Frames | `voice_protocol::signal` |
| Worker death | `Stopped::WorkerDied` in `voice_server/src/main.rs` |
| A socket's place | `Seat` |
| Client | `VoiceCall` in `client/packages/protocol/src/voice.ts` |
| Test server | `voice_protocol/examples/fake_voice_server.rs`, which sends any report or command by hand. The API server's side is tested this way without media. |

Screen sharing and game capture in the client are in [Screen sharing and game capture](../../../client/docs/architecture/screen-sharing-and-game-capture/index.md).

## Joining

1. The client sends `identify` with the join token.
2. The server answers `ready` with the router's RTP capabilities and who is in the call.
3. The client loads a mediasoup device and sends `setCapabilities`.
4. It creates and connects a send and a receive transport.
5. It produces its microphone.

The server creates a paused consumer on the client's receive transport for every producer of everyone else, present and future. Each is announced with `newConsumer` and resumed by the client's `resumeConsumer`.

Every transport assumes a participant can take 10 Mbps before it has measured (`INITIAL_OUTGOING_BITRATE`; mediasoup's default is 600 kbps). A share reaches its viewers sharp from its first seconds, and each receiver's own bandwidth estimate brings it down where it must.

## Producers by source

| Source | Kind | From |
|---|---|---|
| `microphone` | audio | send transport, or `produceRtp` |
| `screen` | video | send transport, or `produceRtp` (game capture) |
| `screenAudio` | audio | send transport (sound the browser captured with the screen), or `produceRtp` (a game's audio) |
| `camera` | video | send transport |

- A screen share is a second producer from the same send transport: `screen` for the picture, `screenAudio` for any sound.
- A camera is a producer of its own. Consumers tell it apart from a screen by its source.
- The grants each source needs are in [Rechecks and grants](rechecks.md#what-the-voice-server-enforces-on-each-producer).

## Plain RTP

### `produceRtp`

A client can ask for a producer it feeds itself.

1. The server creates a plain RTP transport: SRTP, RTP and RTCP multiplexed, the sender's address learned from its first packet.
2. It answers `rtpProduced` with the address, SSRC, payload type, and key.
3. It makes the client a consumer of its own producer, as a preview.

### `consumeRtp`

`consumeRtp` makes the client's receive transport a plain one, answered with `rtpConsuming` (address and key). Every consumer the client is given then sends to it over SRTP.

The benchmark's simulated participants use both, producing `microphone` and `screen` without a browser.

### Game capture

- The desktop shell's game capture sends H.264 through `produceRtp`, straight from its helper to the voice server.
- The game's audio, where the platform can capture it, is a second such producer: source `screenAudio`, Opus, declared `sprop-stereo`.
- One plain transport per producer. Only the video is previewed back to its sender.
- On Linux the picture is an ordinary browser screen producer, and only the game's sound arrives this way, beside it.

## Codecs

The router advertises:

- VP8 and H.264 constrained baseline for video, H.264 listed after VP8,
- Opus for audio.

H.264 is there for the desktop shell's game capture. It is listed after VP8: a browser producing video takes the router's first codec it can send. See [design notes](design-notes.md#media).

`sprop-stereo` is declared on a game's Opus audio because consumers inherit the producer's codec parameters, and a browser decodes Opus as mono without it.

## Mute and deafen

- Mute pauses the microphone producer on the server.
- Deafen pauses every audio consumer. A shared screen stays visible to a deafened participant.
- Mute never touches `screen` or `screenAudio`.
- Both are reported to the API server.

### A moderator's mute

A moderator's mute (`VoiceCommand::Mute`; see [Moderation](moderation.md)) is a flag of its own beside the participant's own mute.

- While it stands, their microphone stays paused, and a new one is made paused.
- A `setState` from their client changes only their own mute.
- The mute everyone is told of is either one: in `participantState`, `ready`, the reports, and so `VoiceParticipant.muted`.
- A moderator's unmute lifts only the moderator's. Someone who muted themself stays muted.

## State reports

- The `participantState` report carries `sharing_screen`. It is sent when a screen producer starts or closes, as well as with mute and deafen changes.
- `VoiceParticipant.sharingScreen` follows it, so a client can mark who is sharing without being in the call.
- Speaking comes from mediasoup's audio level observer (300 ms interval, -50 dBvo).

## Rooms

A call is a room: one router on one worker, created with the first participant and closed with the last. Each step is reported.

- Two joins that start a channel's call at once share the room the first of them made.
- A room closes the moment its last participant leaves. Whoever arrives while it is still being torn down waits until its end is reported, then starts the channel's next call. The API server never hears of the new session before the old one's end, and nobody is let into a room that is about to go.
- Offers and transfers are made under the room's lock with their participants checked present. One is either made before its people leave, and ended as they do, or refused.

### A worker dying

A worker that dies stops the whole server (`Stopped::WorkerDied`), as stopping it would:

1. Everyone is told the server is stopping, so their clients rejoin.
2. The process exits with an error for its supervisor to start it again.

## Sockets and participants

- A user who joins again from another socket replaces their participant.
- A socket leaving takes out only the participant it made. The replaced socket closing never takes out its successor.
- Every frame from a socket acts through the participant it made (`Seat`: the channel, the user, and the connection). It is refused once that participant is gone. A socket never acts for its successor, nor in a later call in the same channel.
- A participant leaving by any path (removed by a moderator or a recheck, replaced, the server stopping) hangs its socket up, after the `kicked` frame saying why. Nothing is left open on a place in the call it no longer holds.
