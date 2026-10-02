# Screen sharing and game capture

- Game capture is the desktop shell's own way to share, through libobs, which reaches games
  that display capture cannot (`game_capture` hooks Direct3D and OpenGL on Windows). The
  helper `packages/desktop/native/obs-capture` (a Rust crate, its own Cargo workspace, built
  by `pnpm build:native` in `packages/desktop`, which can run while a shell is open on Linux
  and macOS) links libobs on Windows (`src/obs/`, behind `cfg(obs)`), and on Linux and
  macOS only when built with its `libobs` feature, for developing the picture path there; a
  Linux build otherwise needs PipeWire's and libopus's development files and no libobs, and a
  macOS build needs only Xcode's toolchain and `cmake` (libopus is built from source). On
  Windows the libobs is the build's own: `pnpm fetch:libobs` (`scripts/fetch-libobs.mjs`) lays
  the OBS Project's release, pinned by version and SHA-256, out in `native/libobs` (gitignored)
  with its files exactly as the OBS Project signed them, since anti-cheat systems whitelist the
  injected game hook by that signature: `obs.dll` and the libraries it and the modules need,
  the five modules the helper loads, their data with the hook's files, headers from the same
  tag's source, and `obs.lib` written from `obs.dll`'s exports (MSVC's `lib` or LLVM's
  `llvm-dlltool`), where the helper's build script finds them; the installer ships that
  directory as the `libobs` resource (`electron-builder.yml`), and the shell starts the helper
  with its libraries on the `PATH` and names its modules and data in every request
  (`bundledLibobs` in `gameCapture.ts`). CI packages the Windows installer that way. With
  the `libobs` feature elsewhere it needs libobs's development files, which on Linux
  `pkg-config` finds and on macOS `LIBOBS_INCLUDE_DIR` and `LIBOBS_LIB_DIR` name: the `libobs`
  directory of an OBS Studio source checkout at the installed version, with an `obsconfig.h`
  written from its `.in` (OBS.app's `Contents/PlugIns` and `Contents/Resources/data` as the
  paths), a directory holding a `libobs.dylib` link to OBS.app's
  `Contents/Frameworks/libobs.framework/Versions/A/libobs` of the same architecture as the
  helper, the helper carrying that Frameworks directory as its rpath, and
  `BINDGEN_EXTRA_CLANG_ARGS=-I/opt/homebrew/include` for `simde` (`brew install simde`),
  which the headers include on ARM; the helper then expects OBS at `/Applications/OBS.app` at
  run time) captures one source, encodes
  it as H.264 constrained baseline, and sends it as SRTP straight to the voice server's plain
  RTP transport, answering the server's RTCP itself. With it goes the captured window's own
  sound: Windows through `wasapi_process_output_capture` and macOS through `sck_audio_capture`
  (both beta OBS features), each pointed at the same window as the picture. A game capture is
  made with the fastest hook rate, so a hook that ran but captured nothing is tried again every
  0.4 s, and one whose hook delivers no frame within `HOOK_TIMEOUT` (three seconds: the game
  never presented, an anti-cheat refused the hook, after which the source never tries again, or
  the window is not a game) is replaced in the scene by a window capture of the same window
  through Windows.Graphics.Capture (`window_capture` with the Windows 10 method,
  `libobs-winrt`), logged on stderr, the sound unchanged. The audio is
  encoded as Opus by obs-ffmpeg and sent as its own SRTP stream to a second producer
  (`produceRtp` with source `screenAudio`), which the sharer does not consume back. The
  helper's RTCP answers: NACKs from a buffer of recent packets, the receiver's bandwidth
  estimate by re-tuning the encoder's bitrate, and keyframe requests by relying on a
  one-second keyframe interval, since libobs cannot be asked for one. The video never passes
  through the shell or the browser, so it is encoded once. All of that is Windows's: on
  macOS, as on Linux, the picture is the browser's own screen share (the system picker on
  macOS 15 and later, which is the same framework libobs's window capture uses, with nothing
  there a hook could add) and the helper captures only the sound, through ScreenCaptureKit
  (`native/obs-capture/src/sck_audio.rs`, which the catalogue names `aspen_sck_app_audio`): a
  stream whose content filter includes the one application, whose audio is then all the
  stream carries besides the smallest, slowest picture the framework allows, handed to the
  same libopus path as Linux's. The framework lists running applications with a window, which
  the catalogue offers as `applicationAudio`, and captures only with the Screen & System
  Audio Recording permission, granted to Aspen in System Settings; the helper's errors say so.
  It is a separate executable, not a
  Node addon, because x264's aligned allocations trip Chromium's allocator in every Electron
  process and because native capture code must not be able to take the app down; its request
  protocol is documented at the top of its `main.rs`. `packages/desktop/src/main/gameCapture.ts`
  spawns it on first use; the preload exposes it as `window.aspenDesktop.gameCapture`. In the
  renderer, `src/features/voice/gameCapture.ts` wraps it as an `ExternalShare` for
  `VoiceCall.startExternalScreenShare`, which asks the voice server for the producers
  (`produceRtp`, answered by `rtpProduced`), hands the targets to the helper, and shows the
  preview the server sends back as a consumer of the call's own producer.
- Linux has no game hook, and its window and screen capture goes through the desktop portal,
  whose picker only the process owning the requesting window can raise, which the helper is
  not. So on Linux, X11 and Wayland alike, a game is shared as a browser screen share (the
  picture, picked in the system's picker) whose sound comes from the helper instead of the
  browser, marked `contentHint: "motion"`: `VoiceCall.startScreenShare({ audio })` takes an `ExternalAudio`, asks the voice
  server for a `screenAudio` RTP producer, and hands its target to it, dropping any sound the
  browser captured. The helper's `startAudio` request captures that sound alone through its
  PipeWire capture (`native/obs-capture/src/pipewire_audio.rs`, which the catalogue names
  `aspen_pipewire_app_audio`): it opens a capture stream and links one application's output
  ports to it, following that application by process id while it runs and by name across a
  restart, so the game keeps playing to its own output while the call hears it, and
  `app_audio.rs` cuts what arrives into 20 ms frames, encodes them as Opus with libopus, and
  sends them as SRTP. The helper advertises no video kinds there (its catalogue says
  `pictures: false`, so the shell offers no test pattern either), and lists the applications
  playing sound as `applicationAudio`.
- `GameCaptureDialog` (opened from `ShareControl`, the share button the call bar and the
  voice channel header both use) follows the platform. On Windows and macOS it lists the
  windows the capture source can be pointed at, with a checkbox for their sound. On Linux it
  lists the applications playing sound as a "Game audio" picker (preselecting the only one when
  just one plays, until the user picks), listing them again twice a second while it is open so
  an application that starts or stops playing appears or goes (a pick stands while its
  application is listed and falls to no audio when it goes), and its button opens the system's picker for the picture. Choosing the sound
  and the picture separately is a known weakness, not a design goal: one choice of "this game"
  is the better experience, and it is out of reach only because the portal tells the app
  nothing about the application behind the window the user picked. Look for ways to close
  that gap: a video track's label names the window on X11, which could identify its
  application, and a future portal may report the window's application. A development shell
  started with `ASPEN_TEST_MEDIA` naming a clip also offers it as a test pattern, looped
  through libobs's media source with its sound; without it no test pattern is offered. A shell without the
  helper on disk shows no game option. The drives run the shell headlessly with
  `ASPEN_DESKTOP_HIDDEN=1`, `ASPEN_DESKTOP_FAKE_MEDIA=1` (with which Chromium answers
  `getDisplayMedia` itself with a synthetic screen, so no picker is shown), and
  `ASPEN_DESKTOP_USER_DATA` pointing at a scratch profile.
- Screen sharing on the desktop shell always goes through a picker. Where the platform has
  one it is used: the desktop portal on Wayland and the system picker on macOS 15 and later.
  Everywhere else the main process lists every screen and window with a thumbnail and the
  renderer shows `SourcePickerDialog` (mounted in `RootLayout`), which answers over the
  `displayPicker` bridge with the chosen source and, on Windows, whether to take the system's
  audio along; dismissing it answers with nothing and the share does not start.
