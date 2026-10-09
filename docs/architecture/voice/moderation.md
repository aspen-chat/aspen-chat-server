# Moderation and server mutes

Holders of Manage calls can server-mute and remove participants. A server mute is the community's and stands across calls.

## Where it lives

| Piece | Code |
|---|---|
| Server mutes | `app::voice::mutes`, table `voice_mute` |
| Command subject | `aspen.voice.command.{server}` |
| Who receives mute events | `events::audience` |
| Rechecks | `rechecks_of` (see [Rechecks](rechecks.md)) |

## Participant endpoints

| Endpoint | Does | Answer |
|---|---|---|
| `PATCH /channels/{channel}/voice/participants/{user}` with `{"muted": bool}` | Server-mutes or unmutes them (makes or lifts the community's mute) | `202 Accepted` |
| `DELETE /channels/{channel}/voice/participants/{user}` | Removes them from the call | `202 Accepted` |

Both answer `202` because the voice server applies the command and its report is what changes the participant record. The caller watches for the `voiceParticipant` event rather than reading the response as the result.

### Who may

- Manage calls.
- As removing and banning do, the participant's highest role must rank below the caller's. Never the owner.
- One who is not a member (a deployment moderator, say) ranks as nobody.
- This is checked before the call is looked up.
- A DM's call has no moderators.

## Server mutes

A server mute stands in every call of the community, joins and rejoins included, until a moderator lifts it. It outlives the person leaving the community, so leaving and coming back does not lift it.

### Endpoints

| Endpoint | Does |
|---|---|
| `PATCH /channels/{channel}/voice/participants/{user}` | Makes or lifts one for someone in the call |
| `PUT /communities/{community}/voice-mutes/{user}` | Makes one, whether or not the person is in a call |
| `DELETE /communities/{community}/voice-mutes/{user}` | Lifts one, whether or not the person is in a call |
| `GET /communities/{community}/voice-mutes` | Lists those standing, newest first, a page of at most `LIST_PAGE` (100) at a time after the mute of `before` (`voice_mute_listed`) |
| `GET /communities/{community}/voice-mutes/{user}` | Reads one person's, not found when none stands. A call's menu learns this way whether someone is muted without reading the whole list. |

All take Manage calls in the community, over someone below the caller's highest role and never the owner. A deployment moderator acting by Moderate any community must outrank them in the deployment too.

### How it reaches the call

1. Making or lifting one publishes `voiceMute` `create` or `delete` inside its transaction, to the community. It reaches holders of Manage calls and the person muted (`events::audience`).
2. The event rechecks the person's calls (`rechecks_of`).
3. Every recheck of a participant in a community's call sends `VoiceCommand::Mute` with the mute as it stands. The voice server leaves it alone when nothing changed.
4. A token issued before a change is brought in line once its join is recorded.

At join time:

- The join offer says whether one stands (`serverMuted`).
- So does the join token (`JoinClaims::server_muted`). The participant it admits is muted from the moment it joins and is reported so.

How a moderator's mute sits beside the participant's own on the voice server is in [Signalling](signalling.md#mute-and-deafen).

### Who observes it

| Who | Through |
|---|---|
| Holders of Manage calls | The list and its events |
| The person | Their join offer, the events, their call |
| Everyone in the call | The participant's `muted` |

### When Manage calls is lost or gained

- A moderator who loses Manage calls stops receiving its events.
- The client drops the list it held (`RecordStore`, as it does bans), and reads it again if the permission returns.

### Client

- The call bar shows its user a mute standing over them (`VoiceCallState.serverMuted`).
- Holders of Manage calls see the community's mutes under its members, each with Unmute.
