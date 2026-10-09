# Media

Code: `VoiceCall` in `packages/protocol/src/voice.ts`. Browser media (`getUserMedia`, a hidden
`<audio>` element per consumer) is in `browserMedia.ts`, behind the `VoiceMedia` interface
(`voiceMedia.ts`). It is loaded lazily so the protocol package stays importable in Node, and tests
pass a fake.

## Consuming

Every `newConsumer` the server announces is consumed:

- Audio plays through a hidden element.
- A screen's video is a `RemoteScreen` in `state.screens`, for the app to render.
- Others' cameras are `state.cameras`, apart from `state.screens`.

## Screen sharing

`startScreenShare()` asks `VoiceMedia.getScreen()`: `getDisplayMedia` with audio, at the screen's
own resolution up to 4K and 60 frames a second (`SCREEN_QUALITY`).

| Producer | Settings |
| --- | --- |
| `screen` (the picture) | One layer, up to 25 Mbps at 60 fps, starting at 10 Mbps (`SCREEN_ENCODING`, `SCREEN_VIDEO_CODEC`). The network's bandwidth estimate brings it down. |
| `screenAudio` (any sound) | Stereo Opus at 128 kbps without DTX (`SCREEN_AUDIO_CODEC`). |

- Its `contentHint` option marks a picture that moves, such as a game, so the encoder gives up
  resolution rather than frames when bandwidth runs short.
- `stopScreenShare()` closes both producers. The share also ends when the browser's own stop
  control ends the track.
- `state.sharingScreen` and `state.localScreen` (the preview track) describe the user's own share.
- In Electron, `getDisplayMedia` only works because the main process answers it in
  `setDisplayMediaRequestHandler` (`packages/desktop/src/main/index.ts`), with a picker. See
  [the screen picker](../screen-sharing-and-game-capture/screen-picker.md).
- Game capture and external audio go through `startExternalScreenShare` and
  `startScreenShare({ audio })`. See [screen sharing and game capture](../screen-sharing-and-game-capture/index.md).

## Cameras

- `startCamera()` asks `VoiceMedia.getCamera()` for the camera chosen in Settings (`video.input`,
  a device preference), up to 1080p at 30 frames a second (`CAMERA_QUALITY`).
- It is produced as `camera`, up to 4 Mbps (`CAMERA_ENCODING`).
- The user sees it as `state.localCamera`. It moves to another camera when the choice changes.
- `stopCamera()` ends it.
- `state.canCamera` says whether the join offer allowed one (Use camera).
- The Android app declares `CAMERA` (and the camera as optional hardware) so its WebView may ask for
  one.

### Camera failures

`getCamera` tries the chosen camera and then every other, so one held by another app does not leave
the user without the rest. A failure is a `CameraError` whose `failure` says why:

| `failure` | Meaning |
| --- | --- |
| `none` | No camera is connected. |
| `denied` | Access was denied. |
| `failed` | Every camera failed. |
| `unsent` | The voice server did not take it. |

It is kept as `state.cameraError` until the camera turns on, `clearCameraError()`, or the call ends.
The call bar shows it under its buttons with what to do.

### Showing cameras

The call screen shows each camera above its owner's name in place of the avatar. The tile is four
columns wide where they fit and the whole row where not. The user's own is mirrored. A camera goes
full screen by its corner button or a double click (`ScreenTile`), but not the F key, which means
the shared screen.

## Signalling frames

The signalling frames are the generated `src/generated/voiceSignal.ts`, from
`voice_signal_schema.json`. `pnpm codegen` builds that schema by running the voice server with
`--gen-signal-schema`.
