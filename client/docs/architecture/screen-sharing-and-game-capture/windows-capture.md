# Windows game capture

The capture pipeline on this page is Windows's. On macOS and Linux the picture is the browser's own
screen share, and the helper captures only the sound (see [application audio](application-audio.md)).

The helper captures one source through libobs, encodes it as H.264 constrained baseline, and sends
it as SRTP straight to the voice server's plain RTP transport, answering the server's RTCP itself.

## The game hook

`game_capture` hooks Direct3D and OpenGL, which reaches games that display capture cannot.

- A game capture is made with the fastest hook rate. A hook that ran but captured nothing is tried
  again every 0.4 s.
- If the hook delivers no frame within `HOOK_TIMEOUT` (three seconds), the capture is replaced in the
  scene by a window capture of the same window. That happens when:
  - the game never presented;
  - an anti-cheat refused the hook (after which the source never tries again);
  - the window is not a game.
- The window capture goes through Windows.Graphics.Capture (`window_capture` with the Windows 10
  method, `libobs-winrt`).
- The replacement is logged on stderr. The sound is unchanged.

## The window's sound

With the picture goes the captured window's own sound, through `wasapi_process_output_capture`
(OBS's "Application Audio Capture", a beta feature), pointed at the same window as the picture.

On macOS, only a helper built with the `libobs` feature, for developing the picture path, captures
through libobs: an application's picture (`screen_capture` type 2) and its sound
(`sck_audio_capture`, likewise beta), both chosen by bundle id. Builds without it capture sound
alone, as [Application audio](application-audio.md) describes.

- It is encoded as Opus by obs-ffmpeg.
- It is sent as its own SRTP stream to a second producer (`produceRtp` with source `screenAudio`).
- The sharer does not consume it back.

## Answering RTCP

| RTCP from the server | The helper's answer |
| --- | --- |
| NACK | Resends from a buffer of recent packets. |
| Receiver's bandwidth estimate | Re-tunes the encoder's bitrate. |
| Keyframe request | Relies on a one-second keyframe interval, since libobs cannot be asked for one. |
