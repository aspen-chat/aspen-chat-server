# The capture helper

The helper `packages/desktop/native/obs-capture` is a Rust crate with its own Cargo workspace.
It is a separate executable, not a Node addon (see [design notes](design-notes.md#the-helper)). Its
request protocol is documented at the top of its `main.rs`.

## What it does on each platform

| Platform | Picture | Sound |
| --- | --- | --- |
| Windows | The helper, through libobs: a game hook, or a window capture (see [Windows game capture](windows-capture.md)). | The captured window's own sound, through libobs. |
| macOS | The browser's own screen share. | The helper, through ScreenCaptureKit (see [application audio](application-audio.md#macos)). |
| Linux (X11 and Wayland) | The browser's own screen share, picked in the system's picker. | The helper, through PipeWire (see [application audio](application-audio.md#linux)). |

On Windows the helper encodes the picture as H.264 constrained baseline and sends it as SRTP
straight to the voice server's plain RTP transport. The video never passes through the shell or the
browser, so it is encoded once.

## How the shell runs it

`packages/desktop/src/main/gameCapture.ts`:

- spawns the helper on first use;
- starts a capture only of a kind the helper's last listing offered (`captureKinds.ts`). The test
  pattern plays only the very clip offered. So the renderer cannot have libobs open a source of any
  other kind;
- on Windows, starts the helper with the bundled libobs's libraries on the `PATH` and names its
  modules and data in every request (`bundledLibobs`; see [building](building-the-helper.md#windows)).

The preload exposes it as `window.aspenDesktop.gameCapture`.

A shell without the helper on disk shows no game option.

## Sharing through it from the renderer

`src/features/voice/gameCapture.ts` wraps the helper as an `ExternalShare` for
`VoiceCall.startExternalScreenShare`. That:

1. asks the voice server for the producers (`produceRtp`, answered by `rtpProduced`);
2. hands the targets to the helper;
3. shows the preview the server sends back, as a consumer of the call's own producer.

The window's sound goes to a second producer (`produceRtp` with source `screenAudio`), which the
sharer does not consume back.

Where the helper supplies only the sound (macOS and Linux), the renderer uses
`VoiceCall.startScreenShare({ audio })` instead (see [application audio](application-audio.md)).
