# Native calls on the phones

In the mobile apps the web view draws the call and nothing more: the call itself runs in the call
engine (`voice_client/` at the repository's root, a Rust crate each app compiles in; `ios-app.md`
and `android-builds.md`, The call engine), which drives the phone's own WebRTC, audio session, and
system call (CallKit, Telecom). The page reaches it through `NativeCall` (`src/api/nativeCall.ts`),
which stands where `VoiceCall` stands in a browser, over the `AspenCall` Capacitor plugin
(`ios/App/App/Call/AspenCallPlugin.swift`, `android/.../call/AspenCallPlugin.kt`). The call's
components read one `VoiceCallState` either way.

## The seam

`VoiceCall`'s public surface is the `Call` interface (`packages/protocol/src/voice.ts`): `state`,
`subscribe`, and every command. `AspenSync.voice` is a `Call`, made by the `createCall` option from
the `VoiceCallOptions` the sync would give `VoiceCall`, which it makes by default. The app's two
places that make a sync (`src/api/sync.tsx` for home, `deployments.tsx` for the others) lay
`nativeCallFactory` over their options, which gives a `createCall` making a `NativeCall` when the
app runs natively and the `AspenCall` plugin is there (`nativeCalls()`), and nothing otherwise. The
factory names each call for the system from the sync's store (`callName`: the channel's name, or a
DM's other people as the DM list names them), which exists only once the sync is made, so the
factory is given a function that answers it later. Nothing in the app imports the `VoiceCall` class;
`useVoiceCall` and everything beneath it read the interface.

## The snapshot

The state crosses as whole snapshots, never as increments. The engine numbers each (`revision`,
counting up with every change) and writes it as JSON beside the record it hands its observer
(`CallObserver::state_changed`'s `json`, `Engine::state_json`); the plugins hand that string to the
page untouched, in a `state` event and as `state()`'s answer, each as `{ origin, state }`
(`PublishedState`), `origin` being the deployment's `public_url`, so each deployment's `NativeCall`
takes its own.

The shape is defined once, in Rust: `voice_client::state::CallState` and the types it holds derive
`serde::Serialize` and `schemars::JsonSchema` beside their UniFFI derives, with camelCase wire
names. `cargo run -p voice_client --bin call-bridge-schema` writes `call_bridge_schema.json` at the
repository root (gitignored, as `voice_signal_schema.json` is), `pnpm codegen` reads it into
`packages/protocol/src/generated/callBridge.ts`, the protocol package exports its root as
`EngineCallState`, and `NativeCall.parseState` reads each published `state` as one. CI's `schemas`
job writes it with the other schemas for the client jobs. Nothing of the snapshot is typed by hand
on either side.

`NativeCall` shows a snapshot when it is its deployment's and its `revision` is past the one shown;
one that arrives late, or again (`state()` read beside an event), changes nothing. The page hears
each snapshot as an event and asks for the current one as it is made and whenever it wakes
(`onPageWake`: `visibilitychange` to visible, Capacitor's `resume`), since a page the system froze
missed those meanwhile. A snapshot that is not the engine's JSON, or holds a value this build's
types lack, is reported (`NativeCallProtocolError`) and shown nothing of.

### What each field becomes

`stateOf` maps `EngineCallState` to `VoiceCallState`, every variant decided by name:

| `VoiceCallState`               | From the snapshot                                                                                                                                                                                                                                                                                |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `status`                       | `status`, the same five names                                                                                                                                                                                                                                                                    |
| `channelId`, `session`         | `channel`, `session`                                                                                                                                                                                                                                                                             |
| `muted`, `deafened`, `held`    | The same; the engine shows a silencing the moment it is asked and a lifting on the server's word, as `VoiceCall` does                                                                                                                                                                            |
| `onHold`                       | The participants whose `held` is set, in id order                                                                                                                                                                                                                                                |
| `serverMuted`                  | The same                                                                                                                                                                                                                                                                                         |
| `canSpeak`                     | `grants.speak`                                                                                                                                                                                                                                                                                   |
| `errorKind`                    | The engine's `offer` (the API server refused the join offer) becomes `server`, which is what `VoiceCall` classes a refused offer as (`failure` in `voice.ts`); the Problem's text is `error` either way, so the call bar reads the same on a phone as in a browser. The other four cross by name |
| `error`, `retryAfterSeconds`   | The same                                                                                                                                                                                                                                                                                         |
| `stateRefused`                 | `{ retryAfterSeconds }` of the engine's `Refusal`                                                                                                                                                                                                                                                |
| `endedReason`                  | The same three names                                                                                                                                                                                                                                                                             |
| `canCamera`                    | `grants.camera`                                                                                                                                                                                                                                                                                  |
| `canShare`, `canTransfer`      | `false`, whatever the grants say: screen sharing from the phone and file transfers are later stages                                                                                                                                                                                              |
| `cameras`, `screens`           | The `videos` of other users (the user's own, their share watched for its preview, left out), each a `RemoteScreen` whose `id` is the engine's track and whose `video` is a `VideoSource` of the `native` kind naming that track as its surface, in the engine's order                            |
| `localCamera`                  | The `camera` among `sources`, as a native surface, facing as the plugin said when it opened the camera (`LocalVideo.facing`, the page's own; the engine carries no facing); `null` when none is sent                                                                                             |
| `sharingScreen`, `localScreen` | Whether a `screen` is among `sources`, and it as a native surface; never set in this stage, since the phone sends no share yet                                                                                                                                                                   |
| `cameraError`                  | `null` while a camera is sent; else `unsent` when `sourceRefused` names the camera (the voice server refused it); else the plugin's last refusal to open one (`LocalVideo.cameraFailure`: `denied`, `none`, or `failed`), kept until the camera opens, `clearCameraError`, or the call ends      |
| `files`                        | The idle state's: nothing is offered                                                                                                                                                                                                                                                             |

## Commands

Each goes to the plugin for the deployment: `join` (with the system's name for the call), `leave`,
`setMuted`, `setDeafened`, `setHeld`, `setServerMuted`, `clearStateRefusal`, `acknowledgeEnd`,
`setUserVolume`, and `onSessionEnded` (the `voiceSessionEnded` event's id, channel, and reason). A
command the plugin refuses is reported (`report`, `console.error` by default); its outcome is
otherwise read from the snapshots. `join` resolves once a snapshot shows the channel connected (or
the call left or taken over by another join meanwhile, as `VoiceCall.join` returns when superseded)
and rejects with `NativeCallFailedError` when one shows it failed.

Volumes: the engine keeps each person's gain for the call and applies it to whichever slot carries
them, so `NativeCall` tells it, as each participant first appears in a snapshot, what `userVolume`
(the sync's: the person's volume under the call's, silent while muted for the user or blocked)
answers for their voice and their share's sound; `refreshVolumes` tells it again for everyone
present; `setUserVolume` sends what it is given; the call's end forgets whom it told.

The camera: `startCamera` asks the plugin (`startCamera` with the camera chosen by the system's id,
or `null` for the phone's default), which asks the system for the camera, opens it, and gives the
engine the track to send; it answers which camera it opened and which way it faces, which the page
keeps as `LocalVideo.facing` for mirroring the preview, and a refusal's code (`denied`, `none`,
`failed`) becomes `cameraError` and a `CameraError` thrown, as `VoiceCall` throws one. Nothing is
asked outside a call or where the camera is not granted, as in `VoiceCall`. `stopCamera` and
`clearCameraError` go to the plugin's methods of those names (the latter also forgetting the page's
own failure). `setAudioDevices` keeps the chosen camera's id for the next start and, while the
camera is on, moves it with the plugin's `switchCamera`, which answers the new facing; the cameras
to choose from are the phone's, listed by the plugin's `cameras()` with each one's facing as the
system says it (`usePhoneCameras` in `useAudioDevices.ts`), so the flip button and the camera list
work as in a browser.

What is shown is what is watched: `showVideo` counts the places showing video, and while there are
any the engine is asked (`setWatched`) to watch every screen and camera another participant sends,
in the participants' order (`watchedOf`), again whenever a snapshot changes that, and none when
nothing shows them; the engine cuts the list to `maxWatchedVideo`. Outside a call nothing is asked,
since the engine keeps nothing to watch.

What a native call has no counterpart for is never a silent no-op:

- Screen sharing (browser and external) and the file transfers reject or throw
  `NativeCallUnavailableError`, naming what is missing; the controls that would ask are hidden by
  the grants above.
- `setAudioDevices` takes the system's default microphone and speaker (the phone's one microphone is
  what is sent, and the route is where the call plays) and rejects either chosen by id, which the
  engine has no counterpart for.
- The microphone is sent as the phone captures it: neither platform's audio device module applies a
  gain over it, so `NativeCall.hasInputVolume` is `false` (`VoiceCall`'s is `true`), the sync never
  calls `setInputVolume` on it (`applyVolumes` in `sync.ts`), the input volume slider is left out of
  Settings and of the mute button's popover, whose arrow goes with it
  ([the call screen](call-screen.md), Device arrows), and `setInputVolume` throws
  `NativeCallUnavailableError` should anything call it.

## The microphone and the system's permission

`VoiceMedia` is never touched: the microphone is the engine's. The plugin's `join` asks the system
for it first (`AVAudioApplication.requestRecordPermission` on iOS 17 and later, `AVAudioSession`'s
before; `RECORD_AUDIO` through Capacitor's permission callback on Android), then opens it through
`CallAudio.openMicrophone` and gives the track to the engine; refused, the engine is given none and
fails the join itself where the channel lets the user speak (`errorKind` `microphone`, which the
call bar shows as the microphone failing), while someone who may not speak joins to listen, as the
web's call does when the browser refuses the microphone. The join then goes through the system's
call (`CallSystem` on iOS, Telecom following the engine on Android), as the engine's own tests do.

## Where the call plays

The Speaker button sets `SPEAKERPHONE`; `useFollowSpeakerphone` (`src/api/audioRoute.ts`) applies it
from the call's start to its end as `AspenCall.setRoute({ kind })`, `loudspeaker` or `receiver`, and
`clearRoute` when the call ends or something else is connected. One path: the plugin gives the
choice to `CallRoutes` (iOS: the override of the output port, applied again on every route change
and when CallKit activates the session; Android: Telecom's `requestEndpointChange`, kept as the
call's `chosen` kind and applied once Telecom lists the call's endpoints), which moves the engine's
own audio. The web view's audio has no route of its own any more: the former `AspenAudioRoute`
plugin, which chose the web view's route, is gone. What the phone can play through comes back the
same way, typed: `AspenCall.routes()` and its `routes` event give each route's kind (`receiver`,
`loudspeaker`, `bluetooth`, `wiredHeadset`, `car`, `other`), the system's name for display alone,
whether it is in use, whether the device has a receiver, and the kind chosen; `usePhoneOutputs`
reads `PhoneOutputs` off them (`outputsOf`), so the Speaker button shows only while a phone has its
receiver and loudspeaker and nothing else to play through, as before.

## Video

The phone's call engine draws every picture natively, behind a transparent web view: the page leaves
a transparent hole where each picture goes and draws names, buttons, popovers, and dialogs over it
as it does over a `<video>`, and nothing is ever drawn above the page. Every picture in the state is
a `VideoSource` of the `native` kind, a surface named by the engine's track id (`media.md`, Showing
cameras), and `ScreenTile` draws one as a hole (`NativePicture`). Three things cross the bridge for
it (`src/features/voice/nativeVideo.ts`):

- **The page reports every hole's rectangle** (`AspenCall.setTiles`, every tile as
  `{ tile, surface, rect, visible, mirrored, cover, fullScreen, pictureInPicture }`): `tile` is the
  page's id for the hole, unique for the page's life, since two tiles may show one surface (a
  share's thumbnail and the focused screen); `rect` is in CSS pixels of the page as they appear on
  screen (the visual viewport's offset and scale applied), which the native side converts to its own
  units; `visible` says whether the hole is on screen at all; `mirrored` mirrors the picture (the
  self preview, while the camera faces its user); `cover` crops the picture to fill the hole rather
  than fitting it whole; `fullScreen` says the tile fills the screen, on which the native side takes
  the screen's own bounds rather than the rectangle, so a turn needs no round trip through the
  page's layout; and `pictureInPicture` names the one picture the system's picture in picture is to
  show. `NativeVideoLayout` measures every hole on an animation frame whenever a hole mounts,
  unmounts, or changes what it shows, whenever a hole's own size changes (`ResizeObserver`), and
  whenever anything on the page may have moved one (a scroll anywhere, a transition or animation
  starting, the window or the visual viewport resizing), and keeps measuring every frame until every
  hole has stood still for two frames, so the last frame of a transition is reported too; an
  unchanged layout is not sent again, and a rectangle is rounded to half a pixel so a settled layout
  never reads as motion. This is the known weak point of the design: the native picture follows the
  page's layout a frame or two behind while a tile moves, so the call screen on a phone pages rather
  than scrolls (`call-screen.md`, On a phone).
- **The native side reports each picture's shape back** (`shape` events,
  `{ surface, width, height, rotation }`, upright: a phone's camera held upright is taller than
  wide, the sender's rotation already applied), as its first frame arrives and whenever it changes,
  which `useNativePictureShape` gives `ScreenTile` in place of a `<video>`'s dimensions.
- **The native side says when the system shows the call as a picture in picture** (`pip` events,
  `{ active }`), which `usePictureInPicture` gives the call screen: on Android the whole page is
  shown small then, so the stage shows the one picture alone; on iOS the system draws the picture
  itself and the page is unaffected.

The holes are cut through the page by `styles.css`: while any hole is on the page the root carries
`data-video-holes`, under which the root and every ancestor of a hole (`[data-video-hole]`) paint no
background, and `NativeVideoBackdrop` (mounted once at the app's root) paints the page's ground
beneath everything with the holes' rectangles cut out as an even-odd `clip-path`, so what was behind
each hole is the native picture and nothing else changes. A hole is a rectangle; a tile's rounded
corners are an opaque frame the tile draws over the hole's corners in the ground's colour, since the
native pictures have no corner radius. Until a picture's first frame the hole shows the native
container behind the web view, which is black, as a `<video>` with nothing to play is.

How each platform draws the surfaces, captures the camera, and shows a picture in picture is
`ios-app.md` and `android-builds.md`, The call engine (Video).

## Sounds

The call's sounds (`callSounds.ts`: joining, leaving, shares, transfers, losing and regaining the
call) stay in the page in this stage, played through Web Audio as everywhere else; the engine takes
them over in a later stage. On the simulator, with the call's audio session configured for a call
(play and record, voice chat) and active, as CallKit has it in a call, the page's `AudioContext`
resumes to `running` and its clock advances (`AspenCallPluginTests` reports both: 0.152 s of clock
over a 300 ms wait in the proof, the web view's context starting up within it), so nothing in the
session's configuration stops the page's sounds; whether they are heard on a device while CallKit
holds the call, and through the route the call plays on, is on the device checklist (`ios-app.md`).

## The stages

The native call's bridge is built in stages, each adding to the same plugin and the same
`NativeCall`; the first two are in place:

1. Voice: join, leave, mute, deafen, hold, server mutes, per-person volume, session endings, the
   routes.
2. Video: the engine's `videos` as surfaces behind a transparent web view, `showVideo` sending
   `watch`, the camera (`startCamera`, `stopCamera`, `cameraError`, the flip), and `canCamera` from
   the snapshot's grants.
3. Screen sharing from the phone (`canShare`, `startScreenShare`).
4. File transfers in the engine (`canTransfer`, `files`, and the offer and transfer commands).
5. The call's sounds and rings in the engine, from the same cues the page derives today.
6. The microphone chosen by device (`setAudioDevices` by id). A gain on it is not planned: the
   phones' audio device modules apply none, which `hasInputVolume` says.

## Proofs

- `src/api/nativeCall.test.ts` (Vitest, a fake plugin): every mapping, parsing, revision ordering,
  the re-read on waking, each forwarded command, the volumes, the join's outcomes, the video (the
  videos as surfaces, the watching following what is shown, the camera's start, facing, failures,
  choice, and stop), the missing input volume, and the rejections. `src/api/audioRoute.test.tsx`
  reads the outputs off the typed routes; `test/sync.test.ts` in the protocol package checks that
  the sync never tells a call without `hasInputVolume` the microphone's volume.
- `AppTests/AspenCallPluginTests` (iOS) and `androidTest`'s `AspenCallPluginTest` (Android) drive
  the real bridge from the app's own page, as `scripts/check_mobile_call.py` runs them beside the
  engine's tests against a deployment of its own (`ios-app.md` and `android-builds.md`, The proof).
