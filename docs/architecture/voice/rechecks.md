# Rechecks and grants

A join token carries the joiner's access as it stood when it was issued. Rechecks bring every call in line when access changes, and the voice server enforces the grants as they now stand.

## Where it lives

| Piece | Code |
|---|---|
| Recheck | `app::voice::recheck` |
| Which calls an event may change | `app::events::rechecks_of` |
| Running rechecks after the work commits | `app::events::noting`, `app::events::settle` |
| Rechecking every call on the deployment | the `recheckAllCalls` job (`app::voice::recheck_all_step`) |
| Sign-in liveness | `login::sign_in_live` |
| Whether an ended sign-in ends a participant | `VoiceCommand::ends_sign_in` |
| Producer kind by source | `MediaSource::kind` |

## When a recheck runs

1. Every event says which calls it may change access to (`rechecks_of`). It matches every event with no wildcard. The events that can change access are:
   - a role's permissions, a role given, taken, or deleted,
   - an override,
   - a channel moved or deleted,
   - an owner named,
   - a member removed, banned, or leaving,
   - a group DM left,
   - a block,
   - a deployment role,
   - a ban,
   - an account deleted,
   - a sign-in ended,
   - a `voiceMute` (see [Moderation](moderation.md#server-mutes)).
2. `publish_event` notes them.
3. Once the request, task, or operator command that published them is done (`noting` and `settle`), the participants of those calls are rechecked:
   - on tasks of their own, at most `RECHECKS_AT_ONCE` (16) at a time on each server;
   - or, for every call on the deployment (turning file transfers on or off), by a job a batch of seats at a time (`recheckAllCalls`; see [Jobs](../jobs/index.md)).
4. Whenever a join's report is applied, the joiner is rechecked. A token issued before a change is brought in line this way.

## What a recheck does

| Finding | Command | Result |
|---|---|---|
| May no longer view the channel or join voice there, or is banned from the deployment, or gone | `VoiceCommand::Kick`, reason `accessLost` | Participant removed; their client tells them |
| Still allowed | `VoiceCommand::Grant` with their grants | See [Grants change](#grants-change) |
| In a community's call | `VoiceCommand::Mute` with the server mute as it stands | Voice server ignores it when nothing changed |

## Ended sign-ins

A sign-in ending (`signInsEnded`, `Recheck::SignIns`) sends each server holding one of the user's calls `VoiceCommand::EndSignIns`. The participant there leaves with reason `signedOut` if the token it joined on was issued to:

- an ended sign-in, or
- no sign-in (a bot's), when every sign-in but one ended (`VoiceCommand::ends_sign_in`). Replacing a bot's token ends every sign-in.

Each join, a replacement from another socket included, is reported with the sign-in its token names. The API server ends it the same way if that sign-in is no longer live (`login::sign_in_live`). A token issued before its sign-in ended and used after it does not keep the participant in the call.

## Grants change

When a `Grant` changes a participant's grants, the voice server:

1. closes the producers they may no longer send,
2. withdraws their offers and ends the transfers they are sending, if they may no longer offer files,
3. tells their client (`grantsChanged`).

The client stops what it may no longer send and opens the microphone it may now send.

## What the voice server enforces on each producer

The voice server refuses a producer when:

- its kind is not its source's kind (`MediaSource::kind`): a microphone and screen audio are audio; a screen and a camera are video;
- it is a microphone producer without `speak`;
- it is a screen or screen audio producer without `share_screen`;
- it is a camera producer without `camera`.

These use the participant's grants as they now stand. They are checked twice:

1. when the frame arrives;
2. again, with the one-producer-per-source rule, under the room's lock as the new producer is added. Making it awaits mediasoup, and a `Grant` may land meanwhile. A `Grant` that lands then finds nothing to close, so the producer is closed instead of added.

A new microphone is likewise brought in line with a mute, a moderator's included, that landed while it was being made. One fed by `produceRtp` is made paused while its sender is muted, as a browser's is.

**Why:** what may be sent and what a mute pauses are decided by source. See [design notes](design-notes.md#producers-by-source).
