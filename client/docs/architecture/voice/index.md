# Voice calls

`AspenSync.voice` is a `VoiceCall` (`packages/protocol/src/voice.ts`): it joins a call on a voice
server, sends and receives its media, and holds the call's state for the app. The app draws the call
bar, the call screen, DM calls and rings, and the per-person and moderation menu on top of it.

## Pages

- [Joining and leaving](joining.md): the join offer, trying candidates, transports and timeouts,
  refusals, moving between calls, rejoining, and how a call ends.
- [Media](media.md): the microphone, screen shares, cameras, playback, and the signalling frames.
- [Participants and volume](participants-and-volume.md): who is in each channel's call, the
  participant list, and per-person volume and muting.
- [Muting and moderation](muting-and-moderation.md): the user's own mute and deafen, server mutes,
  removal, and refused state changes.
- [The call screen and call bar](call-screen.md): `CallBar`, `VoiceScreen`, `CallStage`, and full
  screen.
- [DM calls and rings](dm-calls-and-rings.md): calls in DMs, incoming rings, ringtones, and call
  notices.
- [Design notes](design-notes.md): why the call is built this way.

## Key files

| Part | Where |
| --- | --- |
| `VoiceCall`, `VoiceCallState` | `packages/protocol/src/voice.ts` |
| `VoiceMedia` interface | `packages/protocol/src/voiceMedia.ts` |
| Browser media (`getUserMedia`, `<audio>` elements) | `packages/protocol/src/browserMedia.ts` |
| Signalling frames | `src/generated/voiceSignal.ts` |
| Which participants to show | `src/features/voice/voiceList.ts` |
| Ringtone and dial tone | `loopSound` in `src/features/notifications/sounds.ts` (see [sounds](../notifications.md#sounds)) |
| Joining, leaving, and disconnected sounds | `src/features/voice/callSounds.ts` (see [sounds](../notifications.md#sounds)) |
| Ring notifications | `src/features/voice/ringNotifications.ts` |
| Full screen | `ScreenTile`, `fullScreen.ts`, `useOrientationLock` |
| Desktop screen picker | `packages/desktop/src/main/index.ts` (see [screen sharing](../screen-sharing-and-game-capture/index.md)) |
