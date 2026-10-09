# Screen sharing and game capture: design notes

Rationale behind the [screen sharing and game capture](index.md) pages.

## The helper

See [the capture helper](helper.md).

- **Game capture goes through libobs.** Its `game_capture` hooks Direct3D and OpenGL on Windows, which
  reaches games that display capture cannot.
- **The helper is a separate executable, not a Node addon.** x264's aligned allocations trip
  Chromium's allocator in every Electron process, and native capture code must not be able to take
  the app down.
- **The helper sends SRTP straight to the voice server.** The video never passes through the shell or
  the browser, so it is encoded once.
- **The shell starts only capture kinds the helper's last listing offered.** The renderer cannot have
  libobs open a source of any other kind.
- **Keyframe requests are answered by a one-second keyframe interval.** libobs cannot be asked for a
  keyframe.

## The bundled libobs

See [building the helper](building-the-helper.md).

- **Windows ships the OBS Project's release files exactly as signed.** Anti-cheat systems whitelist
  the injected game hook by that signature.

## Windows capture

See [Windows game capture](windows-capture.md).

- **A hook with no frame within `HOOK_TIMEOUT` falls back to window capture.** The game may never have
  presented, an anti-cheat may have refused the hook (after which the source never tries again), or
  the window may not be a game.

## macOS and Linux

See [application audio](application-audio.md).

- **On macOS the picture is the browser's own share.** The system picker on macOS 15 and later uses
  the same framework libobs's window capture uses, with nothing there a hook could add.
- **On Linux the picture is the browser's own share.** The desktop portal's picker can be raised only
  by the process owning the requesting window, which the helper is not.
- **On Linux the application's ports are linked to the helper's stream.** The game keeps playing to
  its own output while the call hears it.

## The Linux dialog

See [the game capture dialog](game-capture-dialog.md#a-known-weakness).

- **Sound and picture are chosen separately on Linux.** This is a known weakness, not a design goal:
  one choice of "this game" is the better experience. The portal tells the app nothing about the
  application behind the window the user picked.
