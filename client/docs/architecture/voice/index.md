# Voice calls

`AspenSync.voice` is a `Call` (`packages/protocol/src/voice.ts`): in a browser and the desktop app a
`VoiceCall`, which joins a call on a voice server, sends and receives its media over one
`RTCPeerConnection` whose every offer the voice server makes, and holds the call's state for the
app; in the mobile apps a `NativeCall`, which forwards each command to the phone's call engine and
mirrors its state. The app draws the call bar, the call screen, DM calls and rings, and the
per-person and moderation menu on top of either.

## Pages

- [Joining and leaving](joining.md): the join offer, trying candidates, timeouts and refusals, a
  failed connection, resuming, moving between calls, rejoining, and how a call ends.
- [Media](media.md): the call's peer connection and its offers, sending, the microphone, audio slots
  and watched video, screen shares, cameras, and the signalling frames.
- [Participants and volume](participants-and-volume.md): who is in each channel's call, the
  participant list, and per-person volume and muting.
- [Muting and moderation](muting-and-moderation.md): the user's own mute and deafen, server mutes,
  removal, and refused state changes.
- [The call screen and call bar](call-screen.md): `CallBar`, `VoiceScreen`, `CallStage`, and full
  screen.
- [DM calls and rings](dm-calls-and-rings.md): calls in DMs, incoming rings, ringtones, and call
  notices.
- [Native calls on the phones](native-calls.md): the `Call` seam, `NativeCall` over the `AspenCall`
  plugin, the engine's snapshot and its generated type, the microphone permission, the routes, and
  the stages still to come.
- [Design notes](design-notes.md): why the call is built this way.

## Key files

| Part                                                                             | Where                                                                                                                                                       |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Call`, `VoiceCall`, `VoiceCallState`                                            | `packages/protocol/src/voice.ts`                                                                                                                            |
| `NativeCall`, the `AspenCall` plugin's page side                                 | `src/api/nativeCall.ts`, `nativeCallFactory.ts`, `callName.ts`                                                                                              |
| The `AspenCall` plugin                                                           | `ios/App/App/Call/AspenCallPlugin.swift`, Android's `call/AspenCallPlugin.kt`                                                                               |
| `VoiceMedia` interface, `ExternalSender`                                         | `packages/protocol/src/voiceMedia.ts`                                                                                                                       |
| Browser media (`RTCPeerConnection`, `getUserMedia`, `<audio>` elements)          | `packages/protocol/src/browserMedia.ts`, loaded lazily by `platform.ts`                                                                                     |
| Signalling frames                                                                | `src/generated/voiceSignal.ts`                                                                                                                              |
| Calls in real browsers (`scripts/check_calls.py`)                                | `packages/protocol/calls/` (`page.ts`, driven by `drive.ts`)                                                                                                |
| Which participants to show                                                       | `src/features/voice/voiceList.ts`                                                                                                                           |
| Ringtone and dial tone                                                           | `loopSound` in `src/features/notifications/sounds.ts` (see [sounds](../notifications.md#sounds))                                                            |
| The call's sounds (joining, leaving, shares, transfers, losing and regaining it) | `src/features/voice/callSounds.ts` (see [sounds](../notifications.md#sounds)); the engine's own in the mobile apps ([native calls](native-calls.md#sounds)) |
| Ring notifications                                                               | `src/features/voice/ringNotifications.ts`                                                                                                                   |
| Full screen                                                                      | `ScreenTile`, `fullScreen.ts`, `useOrientationLock`                                                                                                         |
| The call bar's device arrows                                                     | `src/features/voice/DeviceArrows.tsx`, `deviceControls.ts`                                                                                                  |
| Receiver or loudspeaker in the mobile apps                                       | `src/api/audioRoute.ts`, over the `AspenCall` plugin's routes                                                                                               |
| Desktop screen picker                                                            | `packages/desktop/src/main/index.ts` (see [screen sharing](../screen-sharing-and-game-capture/index.md))                                                    |
