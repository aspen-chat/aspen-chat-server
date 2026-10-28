# Windows game capture

The capture pipeline on this page is Windows's. On macOS and Linux the picture is the browser's own
screen share, and the helper captures only the sound (see
[application audio](application-audio.md)).

The helper captures one source through libobs, encodes it as H.264 constrained baseline, with the
GPU's encoder where OBS has a module for it and x264 otherwise, and sends it as SRTP straight to the
voice server's plain RTP transport, answering the server's RTCP itself.

## The game hook

`game_capture` hooks Direct3D and OpenGL, which reaches games that display capture cannot.

- A game capture is made with the fastest hook rate. A hook that ran but captured nothing is tried
  again every 0.4 s.
- If the hook delivers no frame within `HOOK_TIMEOUT` (three seconds), the capture is replaced in
  the scene by a window capture of the same window. That happens when:
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

## The encoder

Every encoder produces H.264 constrained baseline, which every browser decodes, with no B-frames, at
a constant bitrate, with a keyframe every second. Which one encodes is decided as the capture starts
(`VIDEO_ENCODERS` in `obs/capture.rs`): the first of these whose module registered it and that
starts.

| Encoder                             | OBS module   | Registered when                                                                      |
| ----------------------------------- | ------------ | ------------------------------------------------------------------------------------ |
| NVIDIA NVENC (`obs_nvenc_h264_tex`) | `obs-nvenc`  | `obs-nvenc-test.exe` finds an NVIDIA GPU with an H.264 encoder.                      |
| Intel Quick Sync (`obs_qsv11_v2`)   | `obs-qsv11`  | `obs-qsv-test.exe` finds an Intel GPU.                                               |
| AMD AMF (`h264_texture_amf`)        | `obs-ffmpeg` | AMD's runtime is installed and `obs-amf-test.exe` finds a GPU with an H.264 encoder. |
| x264 (`obs_x264`)                   | `obs-x264`   | Always.                                                                              |

- A GPU encoder leaves the game its cores. It takes the picture as a texture, with no copy through
  memory, when the picture is rendered on that GPU; rendered on another (a laptop's integrated GPU,
  say), the module reads each frame back and encodes it the same.
- Each module runs its test program as it loads, from beside the helper's executable, where
  `fetch-libobs.mjs` and the installer put them (see
  [building the helper](building-the-helper.md#windows)). Without its program a module registers no
  encoder.
- A module registers its encoder on finding the GPU, but libobs makes the encoder itself as the
  output starts, and one a driver turns down there (no H.264, a GPU already encoding all it can) is
  skipped for the next, with the reason on stderr. The `done` reply to `start` names the encoder in
  use, and the helper logs it.
- `ASPEN_CAPTURE_ENCODER` names one encoder id to use alone, for finding out whether a GPU's encoder
  is what is going wrong.
- NVENC runs preset p4 with the low-latency tuning, one pass, and no lookahead; Quick Sync the
  balanced target usage with one frame in flight; AMF the quality preset, which the module lowers to
  what the GPU keeps up with at the size and rate; x264 veryfast, tuned for zero latency.

## Answering RTCP

| RTCP from the server          | The helper's answer                                                                    |
| ----------------------------- | -------------------------------------------------------------------------------------- |
| NACK                          | Resends from a buffer of recent packets.                                               |
| Receiver's bandwidth estimate | Re-tunes the encoder's bitrate (NVENC answers each change with a keyframe of its own). |
| Keyframe request              | Relies on a one-second keyframe interval, since libobs cannot be asked for one.        |
