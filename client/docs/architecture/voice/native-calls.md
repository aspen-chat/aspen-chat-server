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

| `VoiceCallState`                                                                            | From the snapshot                                                                                                                                                                                                                                                                                |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `status`                                                                                    | `status`, the same five names                                                                                                                                                                                                                                                                    |
| `channelId`, `session`                                                                      | `channel`, `session`                                                                                                                                                                                                                                                                             |
| `muted`, `deafened`, `held`                                                                 | The same; the engine shows a silencing the moment it is asked and a lifting on the server's word, as `VoiceCall` does                                                                                                                                                                            |
| `onHold`                                                                                    | The participants whose `held` is set, in id order                                                                                                                                                                                                                                                |
| `serverMuted`                                                                               | The same                                                                                                                                                                                                                                                                                         |
| `canSpeak`                                                                                  | `grants.speak`                                                                                                                                                                                                                                                                                   |
| `errorKind`                                                                                 | The engine's `offer` (the API server refused the join offer) becomes `server`, which is what `VoiceCall` classes a refused offer as (`failure` in `voice.ts`); the Problem's text is `error` either way, so the call bar reads the same on a phone as in a browser. The other four cross by name |
| `error`, `retryAfterSeconds`                                                                | The same                                                                                                                                                                                                                                                                                         |
| `stateRefused`                                                                              | `{ retryAfterSeconds }` of the engine's `Refusal`                                                                                                                                                                                                                                                |
| `endedReason`                                                                               | The same three names                                                                                                                                                                                                                                                                             |
| `canShare`, `canCamera`, `canTransfer`                                                      | `false`, whatever the grants say: this stage carries voice alone                                                                                                                                                                                                                                 |
| `files`, `sharingScreen`, `localScreen`, `screens`, `localCamera`, `cameras`, `cameraError` | The idle state's: nothing is offered, sent, or watched                                                                                                                                                                                                                                           |

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

What this stage has no native counterpart for is never a silent no-op:

- The camera, screen sharing (browser and external), and the file transfers reject or throw
  `NativeCallUnavailableError`, naming what is missing; the controls that would ask are hidden by
  the grants above.
- `showVideo` returns a function and does nothing: the engine watches no video yet.
- `setAudioDevices` takes the system's defaults (the phone's one microphone is what is sent, and the
  route is where the call plays) and rejects a device chosen by id, which the engine has no
  counterpart for.
- `setInputVolume(1)` is what the engine does (the microphone as captured); any other gain is
  reported, since neither platform's audio device offers a gain over it yet.

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

This is the first stage of the native call's bridge, voice alone; each later one adds to the same
plugin and the same `NativeCall`:

1. Voice (this stage): join, leave, mute, deafen, hold, server mutes, per-person volume, session
   endings, the routes.
2. Video: the engine's `videos` and `share_audio` as opaque surfaces behind a transparent web view,
   `showVideo` sending `watch`, the camera (`startCamera`, `stopCamera`, `cameraError`, the flip),
   and the grants `canCamera` set from the snapshot.
3. Screen sharing from the phone (`canShare`, `startScreenShare`).
4. File transfers in the engine (`canTransfer`, `files`, and the offer and transfer commands).
5. The call's sounds and rings in the engine, from the same cues the page derives today.
6. Microphone gain and device choice (`setInputVolume`, `setAudioDevices` by id).

## Proofs

- `src/api/nativeCall.test.ts` (Vitest, a fake plugin): every mapping, parsing, revision ordering,
  the re-read on waking, each forwarded command, the volumes, the join's outcomes, and the
  rejections. `src/api/audioRoute.test.tsx` reads the outputs off the typed routes.
- `AppTests/AspenCallPluginTests` (iOS) and `androidTest`'s `AspenCallPluginTest` (Android) drive
  the real bridge from the app's own page, as `scripts/check_mobile_call.py` runs them beside the
  engine's tests against a deployment of its own (`ios-app.md` and `android-builds.md`, The proof).
