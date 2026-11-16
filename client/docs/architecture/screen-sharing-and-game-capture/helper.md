# The capture helper

The helper `packages/desktop/native/capture` is a Rust crate with its own Cargo workspace. It is a
separate executable, not a Node addon (see [design notes](design-notes.md#the-helper)). Its request
protocol is documented at the top of its `main.rs`.

## What it does on each platform

| Platform                | Picture                                                                                                        | Sound                                                                                       |
| ----------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Windows                 | The helper, through libobs: a game hook, or a window capture (see [Windows game capture](windows-capture.md)). | The captured window's own sound, through libobs.                                            |
| macOS                   | The browser's own screen share.                                                                                | The helper, through ScreenCaptureKit (see [application audio](application-audio.md#macos)). |
| Linux (X11 and Wayland) | The browser's own screen share, picked in the system's picker.                                                 | The helper, through PipeWire (see [application audio](application-audio.md#linux)).         |

On Windows the helper encodes the picture as H.264 constrained baseline, with the GPU's encoder
where OBS has a module for it and x264 otherwise (see
[Windows game capture](windows-capture.md#the-encoder)), and sends it straight to the voice server
over a WebRTC connection of its own, the capture connection (see [its connection](#its-connection)).
The video never passes through the shell or the browser, so it is encoded once. On macOS and Linux
the sound goes the same way.

## Its requests

The shell writes one JSON request per line to the helper's stdin, and the helper answers each that
has an `id` with one reply line: `done`, `kinds`, or `failed` with a `reason`. One capture runs at a
time.

| Request      | What it does                                                                                                                                                                       | Reply                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `kinds`      | Lists what can be captured now: the capture kinds and their windows, the applications whose sound can be captured alone (`applicationAudio`), and whether it can capture pictures. | `kinds`                                                                          |
| `start`      | Answers the capture connection's first `offer` and starts capturing a picture, with its window's sound when `audio` asks.                                                          | `done` with the connection's `answer` and the `encoder` the picture goes through |
| `startAudio` | Answers the first `offer` and starts capturing one application's sound alone.                                                                                                      | `done` with the `answer`                                                         |
| `answer`     | Answers a later `offer` of the running capture's connection: a section added or taken away, or an ICE restart.                                                                     | `done` with the `answer`                                                         |
| `stop`       | Stops the capture, if one runs, and closes its connection.                                                                                                                         | `done`                                                                           |
| `shutdown`   | Stops any capture and ends the process. Closing stdin does the same.                                                                                                               | none                                                                             |

A `start` or `startAudio` stops any capture already running first. One whose capture does not start
closes the connection it answered.

## How the shell runs it

`packages/desktop/src/main/gameCapture.ts`:

- spawns the helper on first use;
- starts a capture only of a kind the helper's last listing offered (`captureKinds.ts`). The test
  pattern plays only the very clip offered. So the renderer cannot have libobs open a source of any
  other kind;
- reads the helper's replies a line at a time, each settling the request whose id it carries. The
  helper keeps that pipe for replies alone: as it starts it takes the pipe for itself and points its
  stdout at stderr, where libobs's log lines and anything else a library prints go. It answers a
  request it cannot read with a failure, and the shell logs a line that is not a reply
  (`captureReplies.ts`) rather than failing on it;
- on Windows, starts the helper with the bundled libobs's libraries on the `PATH` and names its
  modules and data in every request (`bundledLibobs`; see
  [building](building-the-helper.md#windows)).

The renderer reaches it over IPC: `voice:capture-kinds`, `voice:capture-start`,
`voice:capture-start-audio`, `voice:capture-answer`, and `voice:capture-stop`, each answered with
the helper's reply. Only the renderer that started the capture may answer its offers or stop it, and
the capture stops when that renderer goes away. When the helper exits, the renderer that owned the
capture is sent `voice:capture-ended`. The preload exposes it as `window.aspenDesktop.gameCapture`
(`GameCaptureBridge` in the app's `gameCapture.ts`, declared in `vite-env.d.ts`).

A shell without the helper on disk shows no game option.

## Sharing through it from the renderer

`src/features/voice/gameCapture.ts` wraps the helper as an `ExternalShare` (`gameCaptureShare`) for
`VoiceCall.startExternalScreenShare`, through `helperSender`, the `ExternalSender` both of its
shares are:

1. The call asks the voice server to receive the share's sources (`send` with `external: true`,
   `screen` and, with the window's sound, `screenAudio`).
2. The voice server offers the capture connection, a connection apart from the call's, with a
   `sending` section for each source. It offers H.264 and Opus alone there.
3. The call hands each offer of it to the sender (`ExternalSender.answer`): the first starts the
   capture (`start`, asking for 1920 by 1080 at 60 frames a second and up to 12 Mbps), each later
   one is answered by the running capture (`answer`). The call sends each answer to the voice server
   as the capture connection's. `startExternalScreenShare` resolves once the first is answered, and
   rejects with the helper's failure, the share stopped, when it cannot be.
4. The preview is the voice server's copy of the share: while video is shown, the call watches the
   user's own screen, which comes back on the call connection as a `video` section (see
   [media](../voice/media.md#screen-sharing)). The share's sound is never sent back to the sharer.

The share ends when the user stops it (`stop`), when the helper dies (`onEnded`), and when the voice
server stops receiving it: a capture connection that fails stops its sources there and answers the
`send` that asked for them with an `error` naming it (`refused: "send"`). The call stops a share
whose sources the server never sent on, as it does any refused `send`, and one already sent on when
it sees its own `screen` gone from `sources`. A helper that cannot reach the voice server over UDP,
the only way it tries, so ends its share within the server's connect timeout.

Where the helper supplies only the sound (macOS and Linux), the renderer uses
`VoiceCall.startScreenShare({ audio })` instead (see [application audio](application-audio.md)).

## Its connection

`native/capture/src/rtp.rs` holds the capture connection: a str0m peer that answers the voice
server's offers like any WebRTC client, in str0m's frame mode.

- **The helper reaches the server itself.** It reads the voice server's UDP candidates from the
  first offer, takes the first one this machine has a route to, and binds one UDP socket on the
  interface that route leaves by, which it offers as its only candidate. ICE and DTLS run from there
  on a thread of its own (`aspen-rtc`). **It has no TCP:** a network that lets the browser reach the
  voice server only over TCP cannot carry a game capture, and the voice server gives up on the
  capture connection after its connect timeout (see above for what the client then shows).
- **DTLS and SRTP are str0m's `rust-crypto`**, so no platform's build of the helper needs a C
  toolchain for its crypto.
- **str0m packetizes what the encoders give** (H.264 in Annex B NAL units, Opus packets), one sender
  per stream (`RtpSender`), each finding the `sending` section of its kind with every frame. A frame
  with no section yet, or before the connection is up, is dropped, so a stream whose section comes
  in a later offer starts as soon as that offer is answered.
- **str0m answers NACKs** from what it sent, and writes the sender reports.
- **Every frame is stamped with when it was captured,** on one clock all the capture's streams share
  (`epoch` in `rtp.rs`): the instant its presentation time comes to, from a frame's presentation
  time and the instant it was sent. A frame sent sooner after its capture than any before becomes
  that pairing, so no stamp is in the future. The sender reports follow the stamps, so a viewer
  plays the game's picture and sound in sync, however long each encoder takes.
- **Bandwidth estimation is on**, from the voice server's transport-wide feedback, starting at 6
  Mbps. Its estimate becomes the video encoder's bitrate (`Feedback`), at most every 1.5 seconds,
  between 500 kbps and the bitrate the capture was started with.
- **Keyframe requests are counted**, and logged when the capture stops. libobs cannot be asked for a
  keyframe, so the one-second keyframe interval stands in for answering them.
