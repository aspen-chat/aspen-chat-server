# Media

Code: `VoiceCall` in `packages/protocol/src/voice.ts`. Browser media (`getUserMedia`, the call's
`RTCPeerConnection`, a hidden `<audio>` element per received sound) is in `browserMedia.ts`, behind
the `VoiceMedia` interface (`voiceMedia.ts`). It is loaded lazily (`lazyBrowserMedia` in
`platform.ts`) so the protocol package stays importable in Node, and tests pass a fake.

## The call's connection

- A call holds one peer connection (`VoiceMedia.createConnection`), which carries everything the
  user sends and receives: every media section on one transport (`bundlePolicy: "max-bundle"`,
  `rtcpMuxPolicy: "require"`). It is made before `identify`, so the offer that follows `ready` has a
  connection to answer.
- The voice server makes every offer, and the call only answers. An `offer` frame carries the
  connection (`call`), a `revision`, the SDP, and `sections`, what each media section carries by its
  mid: one of this client's sources (`sending`), an audio slot (`audioSlot`), another participant's
  video (`video`), or a share's sound (`shareAudio`).
- Offers are answered one at a time, in the order they came (`#negotiation`); the server waits for
  each answer before its next offer.
- A revision already answered, which the server restates after a [resume](joining.md#resuming), is
  answered again with the answer kept for it (`#answered`), never applied a second time. A
  `sections` frame, which a resume brings where no offer is outstanding, only says again what each
  section carries.
- The server offers Opus, VP8, and H.264 (constrained baseline) on the call connection. Its offers
  set each section's own format parameters: a sent video's encoder starts at 10 Mbps
  (`x-google-start-bitrate`), and a share's sound is stereo at 128 kbps without DTX.
- An offer the browser cannot apply fails a join in progress as this device's (`CallSetupError`, see
  [joining](joining.md#joining)); in a call, the call is rejoined, since nothing more can be
  negotiated on that connection.

## Sending

- Each source is asked for with `send {source}`. The server's next offer adds a `sending` section
  for it, which the call sets `sendonly` with the source's track before it creates the answer. Once
  the answer is applied, a video's encoding is bounded (`#encode`, `setParameters`): the camera to 4
  Mbps at 30 fps (`CAMERA_ENCODING`), a screen to 25 Mbps at 60 fps (`SCREEN_ENCODING`). These are
  ceilings; the browser's bandwidth estimate decides what is spent.
- A `sending` frame says the server receives the source's media and sends it on. Until it comes the
  source is pending (`#pending`).
- `stopSending` ends a source: its sender's track is taken off at once, and the server takes the
  section away in its next offer.
- Moving to another microphone or camera replaces the sender's track (`replaceTrack`), with no new
  offer.
- A refused `send` (a non-fatal `error` with `refused: "send"`) names the request, not the source,
  so whatever is still pending stops: the camera, with `cameraError: "unsent"`; the share; the
  microphone, with `error` set.

## The microphone

- `getMicrophone` opens the chosen microphone with echo cancellation, noise suppression, and
  automatic gain on, falling back to the default when the chosen one is gone.
- What it captures is sent through a Web Audio gain node, so it goes out at the input volume
  (`INPUT_VOLUME`, 0 to 200%) after the browser's own processing. The call is handed the gain node's
  track, and stopping that track stops the capture too. Without Web Audio the capture is sent as it
  is.
- A stopped microphone's capture is released only once the app's sounds that are not looped have
  ended, starting with any asked for in the same task as the stop, such as the leave sound
  (`soundsEnded` in `soundActivity.ts`, which `playSound` counts sounds for), and at most
  `MICROPHONE_HOLD_LIMIT_MS` (five seconds) after the stop. Nothing is sent or read from it
  meanwhile, though the system shows the microphone in use.
- `VoiceCall.setInputVolume` changes the gain of the open microphone at once, and every microphone
  opens at it. `AspenSync` applies the preference as it changes.
- The microphone is asked for once `ready` arrives, when the user may speak (Speak). It has no
  encoding of its own to bound.

## Receiving

- **Audio slots.** The connection carries one audio slot section for each other microphone in the
  call, up to `ready`'s `audioSlots` (16). It never loses a slot during the call, and has at least
  one. The voice server fills them with whoever is loudest, a muted microphone last, and says whom
  each carries in a `slots` frame, sent before the offer that adds a slot's section (`#slots`). A
  slot that changes speaker keeps its section and its track.
- **A share's sound** comes in a `shareAudio` section of its own while its screen is watched
  (below), never to the sharer themself.
- Each audio slot and share's sound plays by its mid through `VoiceMedia.play` once its section is
  answered, and stops (`VoiceMedia.stop`) when an offer takes the section away. How loud each is
  follows whom it carries (see [participants and volume](participants-and-volume.md)).
- **Video only while something shows it.** `showVideo()` returns a function that undoes it; while
  any call of it stands (the call stage, while mounted; see [the call screen](call-screen.md)), the
  call sends `watch` listing every screen and camera others send, as `ready` and `sources` say, up
  to `ready`'s `maxWatchedVideo`, and the preview of its own share sent from outside the browser.
  With none standing it watches nothing, and no video is received. A `watch` the same as the last is
  not sent again.
- The server adds a `video` section for each video watched and sends it once it has a keyframe. A
  screen is a `RemoteScreen` in `state.screens`, a camera in `state.cameras`, each in the order it
  arrived, its `id` the section's mid. Each keeps its place while its section stays.
- `sources` lists a source only once its media has reached the server, so a camera or screen is
  watched only once there is something to show.

## Screen sharing

`startScreenShare()` asks `VoiceMedia.getScreen()`: `getDisplayMedia` with audio, at the screen's
own resolution up to 4K and 60 frames a second (`SCREEN_QUALITY`).

| Source                    | Settings                                                                                                                                           |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `screen` (the picture)    | One layer, up to 25 Mbps at 60 fps (`SCREEN_ENCODING`), starting at 10 Mbps as the offer says. The connection's bandwidth estimate brings it down. |
| `screenAudio` (any sound) | Stereo Opus at 128 kbps without DTX, as the offer's section says.                                                                                  |

- Its `contentHint` option marks a picture that moves, such as a game, so the encoder gives up
  resolution rather than frames when bandwidth runs short.
- `stopScreenShare()` stops sending both. The share also ends when the browser's own stop control
  ends the track.
- `state.sharingScreen` and `state.localScreen` (the preview track) describe the user's own share. A
  share sent from outside the browser is previewed from the voice server: `localScreen` is then the
  track of a `video` section of the user's own screen, watched while video is shown.
- In Electron, `getDisplayMedia` only works because the main process answers it in
  `setDisplayMediaRequestHandler` (`packages/desktop/src/main/index.ts`), with a picker. See
  [the screen picker](../screen-sharing-and-game-capture/screen-picker.md).
- Game capture and external audio go through `startExternalScreenShare` and
  `startScreenShare({ audio })`. See
  [screen sharing and game capture](../screen-sharing-and-game-capture/index.md).

## Cameras

- `startCamera()` asks `VoiceMedia.getCamera()` for the camera chosen in Settings (`video.input`, a
  device preference), up to 1080p at 30 frames a second (`CAMERA_QUALITY`).
- It is sent as `camera`, up to 4 Mbps at 30 fps (`CAMERA_ENCODING`).
- The user sees it as `state.localCamera`. It moves to another camera when the choice changes and
  stays on: the camera in use is let go before the next opens, since a phone opens one camera at a
  time (iOS ends the first one's track when a second opens). Only the latest of overlapping moves
  takes effect, and where no camera opens the camera turns off with `cameraError` set.
- `stopCamera()` ends it.
- `state.canCamera` says whether the join offer allowed one (Use camera).
- The Android app declares `CAMERA` (and the camera as optional hardware) so its WebView may ask for
  one.

### Camera failures

`getCamera` tries the chosen camera and then every other, so one held by another app does not leave
the user without the rest. A failure is a `CameraError` whose `failure` says why:

| `failure` | Meaning                           |
| --------- | --------------------------------- |
| `none`    | No camera is connected.           |
| `denied`  | Access was denied.                |
| `failed`  | Every camera failed.              |
| `unsent`  | The voice server did not take it. |

It is kept as `state.cameraError` until the camera turns on, `clearCameraError()`, or the call ends.
The call bar shows it under its buttons with what to do.

### Showing cameras

The call screen shows each camera above its owner's name in place of the avatar. The tile is four
columns wide where they fit and the whole row where not, and the camera's picture in it takes the
camera's own shape (`ScreenTile`'s `natural`, read from the picture as its frames arrive), no taller
than 70% of the screen, so tiles are as uneven as the cameras. The user's own is mirrored while it
faces them or its direction is unknown (`trackFacing`: the track's `facingMode`, else its name), and
a phone's back camera is not. A camera goes full screen by its corner button or a double click
(`ScreenTile`), but not the F key, which means the shared screen. Each track plays in a `<video>` of
its own, since WebKit on iOS draws a phone's other camera, put on the element that played the first,
at the first one's rotated size, a portrait picture shrunk inside a black square.

## Signalling frames

The signalling frames are the generated `src/generated/voiceSignal.ts`, from
`voice_signal_schema.json`. `pnpm codegen` builds that schema by running the voice server with
`--gen-signal-schema`.
