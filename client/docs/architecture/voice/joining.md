# Joining and leaving a call

Code: `VoiceCall` in `packages/protocol/src/voice.ts`. State is `VoiceCallState`, read with
`useVoiceCall()`.

## Joining

`join(channelId)` runs these steps:

1. Ask the API server for a join offer: `POST /channels/{channel}/voice/join`.
2. Open the microphone. A refusal fails the join with `errorKind: "microphone"` before any server is
   tried or blamed. Browsers refuse media on an insecure origin, which is anything but `https` or
   `localhost`.
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

- Every request the call waits on an answer to (each transport, its connection, each producer) waits
  at most `READY_TIMEOUT_MS`.
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

The `voiceSessionEnded` event (which `AspenSync` hands to the call) carries a reason. Only the end
of the call's own session (`VoiceCallState.session`, from the `ready` frame) counts: the end of
another session of the channel, as of the dead one a rejoin has already left behind, changes
nothing, so a rejoin is not rejoined again when the API server ends the session it replaced.

| Reason                        | What the client does                                                         |
| ----------------------------- | ---------------------------------------------------------------------------- |
| `serverLost`, `serverRemoved` | Rejoins after a random pause of at most `REJOIN_DELAY_MAX_MS` (a second).    |
| `idle`                        | Sets `endedReason`, which `VoiceEndedDialog` shows until `acknowledgeEnd()`. |
| `empty`                       | The user's own leave.                                                        |

A socket that drops unannounced also rejoins after the same random pause. The API server may not
know yet that the voice server is gone, and offers it again; when the rejoin cannot reach any server
offered (none answers, or the token is refused), the call stays `rejoining`, waits
`DEAD_SERVER_RETRY_MS` (a minute, the server's `offer_silence_seconds`: a voice server silent for
that long is no longer offered, and a call bound to it is ended when someone next asks to join its
channel) and asks for an offer once more, which names a living server. A second failure, or a
failure of any other kind (the offer refused, the microphone, a request turned away, a server not
reached over HTTPS), fails the call at once, as does a failed rejoin after `serverLost` or
`serverRemoved`, whose offer was fresh already.

### `kicked` frames

| Reason           | What the client does                                                                            |
| ---------------- | ----------------------------------------------------------------------------------------------- |
| `kicked`         | Sets `endedReason: "kicked"` for `VoiceEndedDialog`.                                            |
| `accessLost`     | Sets `endedReason: "accessLost"` for `VoiceEndedDialog`. The user may no longer be in the call. |
| `replaced`       | Ends the call silently. Another of the user's own clients took over.                            |
| `signedOut`      | Ends the call silently. The sign-in it joined with ended, which signs this client out too.      |
| `serverStopping` | Rejoins.                                                                                        |
