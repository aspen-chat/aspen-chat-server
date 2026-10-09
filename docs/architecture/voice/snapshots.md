# Snapshots

Some [voice reports](voice-reports.md) can be lost: those sent while NATS is unreachable, those the stream loses when NATS restarts, and those dropped. A voice server's snapshot lets the API server repair its record.

## When a snapshot is sent

- every `SNAPSHOT_INTERVAL_SECONDS` (a minute),
- whenever the voice server reconnects to NATS.

A server's first snapshot waits a full interval after it starts. The calls it held before a restart are carried on by their people rejoining (see [Sessions](sessions.md#when-a-room-is-lost)) rather than ended first.

## What a snapshot holds

1. A `sessionSnapshot` for every call the server holds: its session, its channel, and each participant's mute, deafen, and sharing state.
2. Then a `sessionsHeld` for every lane, empty ones included, listing the server's calls in that lane's channels.

- All parts are made and queued together, so no other report comes between them.
- Each part describes things as of its place in the order.
- Each part goes in its own lane, after every report about those calls sent before it.

## What the API server does with it

The API server makes the record match:

- records a missing session or participant as their own reports would,
- corrects states,
- removes participants not listed,
- ends with reason `serverLost` any session recorded on that server in that lane that `sessionsHeld` leaves out. The database computes the lane of each session's channel the same way the voice server does.

It logs a warning for each repair, since one means a report was lost.

A snapshot's new calls and participants pass the same [offer checks](voice-reports.md#only-where-a-server-was-sent) as `sessionStarted` and `participantJoined`.

## Rooms and snapshots

A room keeps its place until its end is reported, and leaves it in the same step. A snapshot lists a closing room until then, and never after.
