# Calls in DMs

A DM or group DM holds a call as a voice channel does: through the same join, voice servers, and events (see [Voice](../voice/index.md)). Its events are published, as everything in a DM is, to its recipients' user subjects.

## Who may do what

- Everyone in it may join, speak, share a screen, and send files.
- No one moderates it. Moderating a call takes Manage calls, a community permission no one holds in a DM.
- A block between the two people of a one-to-one DM leaves neither able to join its call.

## Rings

Code: `app::voice::ring`, table `voice_ring`.

1. The first person to join becomes `voice_session.started_by`.
2. Everyone else in the DM is rung for `app::voice::ring::RING_SECONDS` (fifteen), but those in do not disturb (see [Presence](../event-routing/presence.md#choosing-a-status)), whom the call reaches only as the DM shows it. Each ring is a row of `voice_ring`, announced as a `voiceRing` create.
3. A ring ends with its `delete` when its person joins the call or declines it (`DELETE /channels/{channel}/voice/rings/@me`).
4. The call's end ends every ring of it.

A ring that runs out:

- Ends at its `until` by every client's clock, with no event.
- The reaper only clears the rows.
- Reads never return a spent one.

The DM list's `include=voice` brings the rings in force too, as `included.voiceRings`.

A session that replaces one lost with its voice server keeps its start and its starter. So its people rejoining rings no one again.

## The call's message

When a DM's call ends, other than by its server being lost, it leaves a message in the DM by whoever started it:

| Kind | When | Carries |
| --- | --- | --- |
| `call` | The call held two people at once | `callSeconds`: how long it lasted |
| `missed_call` | It never held two people at once (`voice_session.had_company`, carried over to a replacing session like the start and the starter) | No length |

Like echoes and poll results, these messages:

- Wake no phone.
- Notify no one.
- Are left out of search.

**Why:** the ring already told everyone of the call.
