# Joining and leaving a call

Code: `VoiceCall` in `packages/protocol/src/voice.ts`. State is `VoiceCallState`, read with
`useVoiceCall()`.

## Joining

`join(channelId)` runs these steps:

1. Ask the API server for a join offer: `POST /channels/{channel}/voice/join`.
2. Open the microphone. A refusal fails the join with `errorKind: "microphone"` before any server is
   tried or blamed. Browsers refuse media on an insecure origin, which is anything but `https` or
   `localhost`.
3. Ping each candidate voice server's `/health` at once, and try them nearest first. A candidate not
   reached over HTTPS is never tried, nor reported.
4. Make the call's peer connection (`VoiceMedia.createConnection`; see
   [media](media.md#the-calls-connection)).
5. Open the candidate's signalling socket and send `identify` with the offer's token. A candidate
   whose socket does not open, that refuses the token, or that does not answer `ready` within
   `READY_TIMEOUT_MS` is reported to `POST /voice-servers/{server}/failures`, and the next one is
   tried.
6. Once a server answers `ready`, ask to send the microphone (`send`), and answer each offer the
   server makes.
7. Wait for the peer connection's ICE and DTLS to connect (`CONNECT_TIMEOUT_MS`, 15 seconds). A
   connection that fails or times out counts as that server's failure: it is reported and the next
   candidate is tried. **Why:** the server accepting the call says nothing about media
   ([design notes](design-notes.md#joining)).

Only a failure known to be the server's own (`VoiceServerError`) is reported and followed by the
next candidate: its socket not opening, a fatal `error` frame, `ready` never coming, media that does
not connect. Anything else thrown while setting the call up (the browser without WebRTC, a peer
connection it could not make, an offer it could not apply, such as one whose codecs it lacks) is
this device's and would meet every server alike, so the join fails at once as a `CallSetupError`,
with `errorKind: "device"`, and no server is reported.

## Timeouts and refusals

- `identify` and `resume` wait at most `READY_TIMEOUT_MS` for `ready`. Nothing else the call sends
  waits for an answer: what it asks for arrives as the server's next offer, or as a refusal.
- A fatal `error` at `identify` that carries `retryAfterSeconds` (the voice server turned the user
  away for going too fast) fails the join as a `VoiceRequestRefused`, with `errorKind: "refused"`,
  and the call bar says how long to wait. A refusal is not reported as the server's failure, and no
  other server is tried (unlike a timeout).
- In a call, the voice server refuses a request it will not honour with a non-fatal `error` naming
  it (`refused`): a refused `send` stops what waited to be sent (see [media](media.md#sending)), and
  a refused `setState` leaves the user muted or deafened (see
  [muting](muting-and-moderation.md#refused-state-changes)).
- A fatal `error` mid-call fails the call, with its `detail` as `error`.

## When the connection fails

- A peer connection that goes `failed` mid-call asks for an ICE restart (`restartIce`), which the
  server answers with an offer of new ICE credentials. One still not connected `CONNECT_TIMEOUT_MS`
  later is rejoined. The server offers a restart of its own when it sees ICE disconnect, which
  recovers a network change before the browser gives up on it.
- An offer the browser cannot apply mid-call rejoins.
- A `kicked` frame with `connectionFailed` (the server never heard this client's media, or lost it)
  rejoins.

## Resuming

A signalling socket that closes on its own, without `leave`, leaves the call in place at the voice
server for twenty seconds, its media still flowing: the peer connection does not depend on the
socket. The call opens a socket to the same server and sends `resume` with the secret the last
`ready` gave (`#resume`).

- It tries `RESUME_ATTEMPTS` (three) times, the first at once and each after waiting a random time
  of up to `RESUME_DELAY_MAX_MS` (a second) more than the one before, all well inside the server's
  grace.
- A server that answers `ready` takes the call back and restates each connection's offer: the one
  outstanding, which the call answers (or answers again; see
  [media](media.md#the-calls-connection)), or else `sections`. The call then sends its `watch` and
  its mute and deafen again.
- A socket that opened and refused the secret will not take the call back, and is not tried again.
  When no attempt succeeds the call is rejoined (see [session endings](#session-endings)).

## Moving between calls

Joining another call while in one leaves the first and joins the second at once, with a peer
connection of its own. **The microphone stays open**: the call holds its track apart from the
connection, so closing the connection does not stop it, and the next call sends the same track. It
is closed when the next call only lets the user listen.

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

A signalling socket that closes on its own and cannot be [resumed](#resuming) rejoins after the same
random pause. The API server may not know yet that the voice server is gone, and offers it again;
when the rejoin cannot reach any server offered (none answers, or the token is refused), the call
stays `rejoining`, waits `DEAD_SERVER_RETRY_MS` (a minute, the server's `offer_silence_seconds`: a
voice server silent for that long is no longer offered, and a call bound to it is ended when someone
next asks to join its channel) and asks for an offer once more, which names a living server. A
second failure, or a failure of any other kind (the offer refused, the microphone, a request turned
away, a server not reached over HTTPS), fails the call at once, as does a failed rejoin after
`serverLost` or `serverRemoved`, whose offer was fresh already.

### `kicked` frames

| Reason             | What the client does                                                                            |
| ------------------ | ----------------------------------------------------------------------------------------------- |
| `kicked`           | Sets `endedReason: "kicked"` for `VoiceEndedDialog`.                                            |
| `accessLost`       | Sets `endedReason: "accessLost"` for `VoiceEndedDialog`. The user may no longer be in the call. |
| `replaced`         | Ends the call silently. Another of the user's own clients took over.                            |
| `signedOut`        | Ends the call silently. The sign-in it joined with ended, which signs this client out too.      |
| `serverStopping`   | Rejoins.                                                                                        |
| `connectionFailed` | Rejoins. The server never heard this client's media, or lost it.                                |
