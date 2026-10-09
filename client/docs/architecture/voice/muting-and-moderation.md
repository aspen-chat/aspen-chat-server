# Muting and moderation

## The user's own mute and deafen

- Muting and deafening show at once and disable the microphone's track locally
  (`#holdMicrophone`, which keeps it disabled whenever `muted` is shown). The voice server never
  refuses them.
- Unmuting and undeafening are only asked for (`#asked`). They show once a `participantState` frame
  about the user themself says the server did them.
- That frame sets `muted` and `deafened` in the call state. This is how a server mute shows on the
  user's own controls.
- The frame never shows the user less silenced than they have since asked to be. **Why:** an answer
  to an earlier request must not reopen a microphone they just muted.

### Refused state changes

A non-fatal `error` with `refused: "setState"`:

- leaves the user as shown;
- sets `stateRefused` (with any `retryAfterSeconds`).

The call bar shows it under its buttons until the user tries again, dismisses it
(`clearStateRefusal`), or the call ends.

## Moderation

Moderation lives in `ParticipantMenu` (see [participants and volume](participants-and-volume.md)).

| Action | Call | Notes |
| --- | --- | --- |
| Server mute or unmute | `AspenSync.muteVoiceParticipant` | `202 Accepted`. The mute is the community's and outlasts the call. |
| Remove | `kickVoiceParticipant` | `202 Accepted`. |

Both actions' effects arrive as the participant's own events.

### Server mutes

- The menu reads the mute from the community's mutes (`useVoiceMuted`).
- Holders of Manage calls also see and lift these mutes under the community's members
  (`VoiceMutedList`).
- The muted user's call bar shows it from `VoiceCallState.serverMuted`. The join offer and the
  `voiceMute` events about them set it.

In a DM no one is offered moderation, since the resolver gives no one Manage calls in a DM.

A `kicked` frame's effects are under [how a call ends](joining.md#how-a-call-ends).
