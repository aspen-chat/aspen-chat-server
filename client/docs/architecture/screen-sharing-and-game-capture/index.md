# Screen sharing and game capture

Game capture is the desktop shell's own way to share, through a native helper that uses libobs. It
reaches games that display capture cannot. On every platform the desktop shell's ordinary screen
share goes through a picker.

## Pages

- [The capture helper](helper.md): what the helper is, how the shell runs it, and how the renderer
  shares through it.
- [Building the helper](building-the-helper.md): build requirements per platform, the bundled libobs
  on Windows, and the `libobs` feature elsewhere.
- [Windows game capture](windows-capture.md): the game hook, its fallback to window capture, the
  window's sound, and answering RTCP.
- [Application audio on macOS and Linux](application-audio.md): the browser's picture with the
  helper's sound, through ScreenCaptureKit and PipeWire.
- [The game capture dialog](game-capture-dialog.md): `GameCaptureDialog` per platform, the test
  pattern, and running the shell headlessly.
- [The screen picker](screen-picker.md): how the desktop shell picks a screen or window.
- [Design notes](design-notes.md): why capture is built this way.

## Key files

| Part | Where |
| --- | --- |
| Helper (Rust, own Cargo workspace) | `packages/desktop/native/obs-capture` |
| Helper's request protocol | top of the helper's `main.rs` |
| libobs bindings (Windows) | `native/obs-capture/src/obs/` |
| macOS sound | `native/obs-capture/src/sck_audio.rs` |
| Linux sound | `native/obs-capture/src/pipewire_audio.rs`, `app_audio.rs` |
| Fetching libobs | `scripts/fetch-libobs.mjs` (`pnpm fetch:libobs`) |
| Main process side | `packages/desktop/src/main/gameCapture.ts`, `captureKinds.ts` |
| Renderer side | `src/features/voice/gameCapture.ts` |
| Dialogs | `GameCaptureDialog`, `ShareControl`, `SourcePickerDialog` |
