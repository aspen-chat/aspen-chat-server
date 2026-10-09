# Application audio on macOS and Linux

On macOS and Linux the picture is the browser's own screen share, and the helper captures only one
application's sound.

## macOS

- The picture is the browser's own screen share: the system picker on macOS 15 and later. That is the
  same framework libobs's window capture uses, with nothing there a hook could add.
- The helper captures the sound through ScreenCaptureKit (`native/obs-capture/src/sck_audio.rs`,
  which the catalogue names `aspen_sck_app_audio`).
- The stream's content filter includes the one application. The stream then carries only its audio,
  besides the smallest, slowest picture the framework allows.
- The audio is handed to the same libopus path as Linux's.
- The framework lists running applications with a window. The catalogue offers them as
  `applicationAudio`.
- **It captures only with the Screen & System Audio Recording permission**, granted to Aspen in System
  Settings. The helper's errors say so.

## Linux

Linux has no game hook. Its window and screen capture goes through the desktop portal, whose picker
only the process owning the requesting window can raise. The helper is not that process.

So on Linux, X11 and Wayland alike, a game is shared as a browser screen share whose sound comes from
the helper instead of the browser:

1. The picture is picked in the system's picker, and marked `contentHint: "motion"`.
2. `VoiceCall.startScreenShare({ audio })` takes an `ExternalAudio`.
3. It asks the voice server for a `screenAudio` RTP producer and hands its target to the
   `ExternalAudio`.
4. Any sound the browser captured is dropped.
5. The helper's `startAudio` request captures the application's sound alone.

### The PipeWire capture

`native/obs-capture/src/pipewire_audio.rs`, which the catalogue names `aspen_pipewire_app_audio`:

- opens a capture stream and links one application's output ports to it;
- follows that application by process id while it runs, and by name across a restart.

The game keeps playing to its own output while the call hears it.

`app_audio.rs` cuts what arrives into 20 ms frames, encodes them as Opus with libopus, and sends them
as SRTP.

### What the helper offers on Linux

- No video kinds. Its catalogue says `pictures: false`, so the shell offers no test pattern either.
- The applications playing sound, as `applicationAudio`.
