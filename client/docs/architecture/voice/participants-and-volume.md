# Participants and volume

## Who is in each channel's call

Who is in each channel's call is store state:

| Part | Name |
| --- | --- |
| Store | `RecordStore.channelVoice` |
| Topic | `voice:<channelId>` |
| Hook | `useChannelVoice` |
| Built from | the `voice` sideload and the `voiceSession`, `voiceParticipant`, and `voiceSpeaking` events |

- Each participant carries `speaking` and `lastSpokeAt`.
- `src/features/voice/voiceList.ts` picks the fifteen to show under a channel. Once a call is larger
  than that, the most recent speakers come first.
- `VoiceParticipants` draws them, with a green ring while speaking and a monitor mark while sharing.

## Clicking and right-clicking a person

- Clicking a person shows their `ProfilePopover` beside the row (anchored to the row, not the name).
  Clicking them again, or anywhere else, closes it.
- Right-clicking a person, or the dots beside them, opens `ParticipantMenu`. It holds:
  - how loud they are to this user alone;
  - "mute for me";
  - while they share a screen, the same two for its sound apart from their voice;
  - the moderation actions (see [muting and moderation](muting-and-moderation.md)).

## Per-person volume and muting

Volume and muting here are for this user alone. Nothing the server or the other person can see.

| Control | Device preference | `AspenSync` setter | Applied as | To consumers |
| --- | --- | --- | --- | --- |
| Voice volume | `userVolume(userId)`, a gain from 0 to `MAX_USER_VOLUME` | `setUserVolume` | `effectiveUserVolume` | `microphone` |
| Voice "mute for me" | `userMuted(userId)` (keeps their volume) | `setUserMuted` | `effectiveUserVolume` | `microphone` |
| Stream volume | `streamVolume` | `setStreamVolume` | `effectiveStreamVolume` | `screenAudio` |
| Stream mute | `streamMuted` | `setStreamMuted` | `effectiveStreamVolume` | `screenAudio` |

`VoiceCall` asks `userVolume(user, source)` for the gain of each consumer.

### Playback through Web Audio

- Playback runs through a Web Audio gain node, since an element's own volume stops at 1.
- Each participant's raw stream also plays through a muted element (`browserMedia.ts`). Chromium,
  and so the desktop app, feeds a remote WebRTC track into Web Audio only while the track also plays
  through a media element. Without it the graph is silent there. Firefox plays either way.
