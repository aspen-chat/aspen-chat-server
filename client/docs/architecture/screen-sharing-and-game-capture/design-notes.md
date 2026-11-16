# Screen sharing and game capture: design notes

Rationale behind the [screen sharing and game capture](index.md) pages.

## The helper

See [the capture helper](helper.md).

- **Game capture goes through libobs.** Its `game_capture` hooks Direct3D and OpenGL on Windows,
  which reaches games that display capture cannot.
- **The helper is a separate executable, not a Node addon.** x264's aligned allocations trip
  Chromium's allocator in every Electron process, and native capture code must not be able to take
  the app down.
- **The helper sends straight to the voice server, over a WebRTC connection of its own.** The video
  never passes through the shell or the browser, so it is encoded once. The connection is negotiated
  like any client's, the voice server offering and the helper answering through the renderer, so the
  server has one way of negotiating to build and test.
- **The helper is a str0m peer in frame mode.** str0m packetizes H.264 and Opus, answers NACKs, and
  writes sender reports, which the helper would otherwise write itself; its `rust-crypto` feature
  keeps DTLS and SRTP free of C on every platform the helper builds for.
- **Frames are stamped with their capture time, not the instant they are sent.** The picture and the
  sound are encoded on threads of their own, at delays of their own, and a viewer synchronizes the
  two by the sender reports' times, so stamping each frame as it is sent would put the difference
  between the encoders' delays between picture and sound. str0m's reports give no RTP time for a
  stream whose last frame is stamped in the future, so the clock is paired with the quickest frame
  seen rather than the first.
- **Its bandwidth estimate steers the encoder.** The voice server estimates no bandwidth for what it
  receives, so the sender's own estimate, from the server's transport-wide feedback, is what brings
  a game's bitrate down on a narrow uplink, as a browser's does for a browser share.
- **The helper speaks UDP alone.** This is a known weakness, not a design goal: it reaches the
  server from one socket on the interface its route takes, and has no TCP, so a network that blocks
  UDP to the voice server leaves game capture without a way through, though the call itself goes on
  over the voice server's TCP candidates.
- **The shell starts only capture kinds the helper's last listing offered.** The renderer cannot
  have libobs open a source of any other kind.
- **Keyframe requests are answered by a one-second keyframe interval.** libobs cannot be asked for a
  keyframe; the helper counts the requests and logs how many came.

## The bundled libobs

See [building the helper](building-the-helper.md).

- **Windows ships the OBS Project's release files exactly as signed.** Anti-cheat systems whitelist
  the injected game hook by that signature.

## Windows capture

See [Windows game capture](windows-capture.md).

- **A hook with no frame within `HOOK_TIMEOUT` falls back to window capture.** The game may never
  have presented, an anti-cheat may have refused the hook (after which the source never tries
  again), or the window may not be a game.
- **The GPU's encoder is tried before x264.** The game and x264 want the same cores, and a GPU
  encoder takes the picture as a texture. Every encoder is set to the same H.264 constrained
  baseline, no B-frames, constant bitrate, and one-second keyframes, so the voice server and every
  viewer see one stream whichever encodes it.
- **NVIDIA, then Intel, then AMD, then x264.** Each GPU module registers its encoder only when its
  test program finds that vendor's GPU, so with one GPU the order decides nothing; with two, the
  first vendor present wins, and each module copes with a picture rendered on the other adapter by
  reading it back. x264 is always there.
- **The encoder is chosen as the output starts, not when its module loads.** A module that found the
  GPU registers its encoder, but libobs makes the encoder itself as the output starts, and that is
  where a driver turns one down; the helper then tries the next.
- **The encoders' test programs sit beside the helper, not under `libobs/`.** libobs runs them from
  beside the executable that loaded it.

## macOS and Linux

See [application audio](application-audio.md).

- **On macOS the picture is the browser's own share.** The system picker on macOS 15 and later uses
  the same framework libobs's window capture uses, with nothing there a hook could add.
- **On Linux the picture is the browser's own share.** The desktop portal's picker can be raised
  only by the process owning the requesting window, which the helper is not.
- **On Linux the application's ports are linked to the helper's stream.** The game keeps playing to
  its own output while the call hears it.

## The Linux dialog

See [the game capture dialog](game-capture-dialog.md#a-known-weakness).

- **Sound and picture are chosen separately on Linux.** This is a known weakness, not a design goal:
  one choice of "this game" is the better experience. The portal tells the app nothing about the
  application behind the window the user picked.
