# Joining and leaving a call

Code: `VoiceCall` in `packages/protocol/src/voice.ts`. State is `VoiceCallState`, read with
`useVoiceCall()`.

## Joining

`join(channelId)` runs these steps:

1. Ask the API server for a join offer: `POST /channels/{channel}/voice/join`.
2. Open the microphone. A refusal fails the join with `errorKind: "microphone"` before any server
   is tried or blamed. Browsers refuse media on an insecure origin, which is anything but `https`
   or `localhost`.
3. Ping each candidate voice server's `/health` at once, and try them nearest first.
4. A candidate that refuses the token, or does not answer within `READY_TIMEOUT_MS`, is reported to
   `POST /voice-servers/{server}/failures`, and the next one is tried.
5. Once a server answers `ready`, load a mediasoup `Device`.
6. Ask for a send and a receive transport at once.
7. Produce the microphone.
8. Wait for the send transport's ICE and DTLS to connect (`CONNECT_TIMEOUT_MS`). A transport that
   fails or times out counts as that server's failure: it is reported and the next candidate is
   tried. **Why:** the server accepting the producer says nothing about media
   ([design notes](design-notes.md#joining)).

A transport that fails mid-call rejoins.

## Request timeouts and refusals

- Every request the call waits on an answer to (each transport, its connection, each producer)
  waits at most `READY_TIMEOUT_MS`.
- An `error` frame arriving first refuses the request. The voice server answers a request it will
  not honour with one. While a join's requests are in flight, that is the one refused (`refused`
  names its type).
- A refused request fails the join as a `VoiceRequestRefused`, with `errorKind: "refused"`.
- When the voice server turned the user away for going too fast, it carries `retryAfterSeconds`. A
  fatal `error` at `identify` with a wait counts the same. The call bar says how long to wait.
- A refusal is not reported as the server's failure, and no other server is tried (unlike a
  timeout).

## Moving between calls

Joining another call while in one leaves the first and joins the second at once. What does not
depend on the call carries over:

- **The microphone stays open.** Its producer is made with `stopTracks: false`, so the call, not the
  transport, decides when the track stops. It is closed when the next call only lets the user
  listen.
- **The loaded `Device` is kept**, by the router capabilities it was loaded with. Every call on a
  voice server shares them, so it is loaded once rather than for every call.

A rejoin keeps the microphone the same way.

## How a call ends

`VoiceCallState` keeps `channelId` while `failed`, so the call bar can show why.

### Session endings

The `voiceSessionEnded` event (which `AspenSync` hands to the call) carries a reason:

| Reason | What the client does |
| --- | --- |
| `serverLost`, `serverRemoved` | Rejoins after a random pause of at most `REJOIN_DELAY_MAX_MS` (a second). |
| `idle` | Sets `endedReason`, which `VoiceEndedDialog` shows until `acknowledgeEnd()`. |
| `empty` | The user's own leave. |

A socket that drops unannounced also rejoins after the same random pause.

### `kicked` frames

| Reason | What the client does |
| --- | --- |
| `kicked` | Sets `endedReason: "kicked"` for `VoiceEndedDialog`. |
| `accessLost` | Sets `endedReason: "accessLost"` for `VoiceEndedDialog`. The user may no longer be in the call. |
| `replaced` | Ends the call silently. Another of the user's own clients took over. |
| `signedOut` | Ends the call silently. The sign-in it joined with ended, which signs this client out too. |
| `serverStopping` | Rejoins. |
