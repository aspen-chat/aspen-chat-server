# Choosing a server

Join voice offers candidate servers. The client measures its latency to each and tries them nearest first. A server that fails to start a session can be reported, and enough reports suspend it.

## Settings

All under `[voice]` in `aspen.toml`:

| Setting | Default | Use |
|---|---|---|
| `candidate_limit` | ten | Most candidates in one offer |
| `offer_silence_seconds` | a minute | A server silent this long is not offered |
| `failure_threshold` | five | Distinct users whose reports suspend a server |
| `failure_window_seconds` | an hour (3600) | Window reports are counted in, and how long a suspension lasts |

## Candidates

- A channel already in a call is offered only that call's server.
- Otherwise every candidate is a server that:
  - is enabled,
  - has room,
  - has reported within `offer_silence_seconds`. A server that has never reported has not started.
- At most `candidate_limit` are chosen, at random for now.
- A call bound to a server silent for `offer_silence_seconds` is skipped when someone joins its channel. They get fresh candidates.

Each offer also notes in Valkey which servers it named, for the [report checks](voice-reports.md#where-a-report-came-from) and the failure checks below.

## Client side

The client (`VoiceCall` in `client/packages/protocol/src/voice.ts`):

1. pings every candidate's `GET /health` (CORS open),
2. tries them nearest first,
3. reports a server that fails to start its session.

## Failure reports

`POST /voice-servers/{server}/failures` reports a server that failed to start the session.

1. Reports count distinct users within `failure_window_seconds`.
2. At `failure_threshold` the server is suspended for `failure_window_seconds` (`voice_server.suspended_until`, shown as `suspendedUntil`).
3. After that it is offered again on its own: its failures are all older than the window by then.

Each report deletes its server's failures older than the window. The recurring job `sweepExpired` deletes those of servers no longer reported (see [Job kinds](../jobs/kinds.md)).

- An operator enabling or disabling a server ends a suspension. Enabling it forgets its failures.
- The suspension is decided with every server's row locked.
- A server is never suspended when no other would be left taking calls. Throwaway accounts can at worst suspend every server but one.

### Which reports count

A report counts only from one of this deployment's people who was offered the server and has not joined a call there within the window.

| Valkey key | Written by | Kept for |
|---|---|---|
| `voice_offered:{server}:{user}` | each join offer, for each candidate | the token's lifetime and a minute more |
| `voice_joined:{server}:{user}` | each applied join report | the window |

A report is answered `counted: false` and changes nothing when:

- the first key is missing,
- the second key is present,
- it comes from a bot, or
- it comes from a foreign user.

**Why:** no one can suspend a server they were never sent to or reached. See [design notes](design-notes.md#failure-reports).

A Valkey outage leaves offers unnoted and is read as having joined, so reports then count for nothing.
