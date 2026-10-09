# Calls

## Nobody can join a call

People see "No voice server can take a call right now". No voice server is enabled, has room, and
reported within `[voice] offer_silence_seconds`.

The dashboard's Server fleet tab shows each voice server's last report. Check that the voice
server:

1. is running;
2. reaches NATS, with the token, or as its own user with the permissions
   [Step 6: Voice servers](../installing/6-voice-servers.md#give-it-a-nats-user) lists. NATS logs a
   `Permissions Violation` for anything else;
3. has the `id` the registry gave it (see
   [A voice server shows as Silent](#a-voice-server-shows-as-silent-though-it-is-running)).

An API server logs one of these for a report about another voice server's calls:

- `a voice report on a subject it does not belong on was dropped`
- `a voice report about what is not that server's was dropped`

## A voice server shows as Silent though it is running

Or it stops at startup saying `this voice server's id is not registered`.

**Means:** its `id` in `voice_server.toml` (or `ASPEN_VOICE_SERVER_ID`) is not the id the
deployment registered it under. It was:

- copied from another deployment,
- kept from before the database was recreated, or
- kept after the server was removed and registered again, which gives it a new id.

Its reports name an id no registered server has, so they are dropped. The Server fleet tab lists
the id it reports as under **Voice servers that are not registered**, and the API servers log
`a voice server whose id is not registered is reporting`.

**What to do:**

1. Set `id` to the registered server's id (`voice-servers list`, or the ID column of the Server
   fleet tab).
2. If it has a NATS user of its own, change the id in that user's permissions to match.
3. Restart it.

A voice server checks its id each time it starts and stops when it is wrong. So one that keeps
running unregistered was removed while it ran, or its API servers are older than it.

## People join a call but hear nothing

Signalling works but media does not flow. Either:

- the media ports (`[rtc] min_port` to `max_port`, UDP and TCP) are closed, or
- `[rtc] announced_address` is not the address clients can reach. Behind NAT it must be the
  public one.

## A voice server's snapshot repaired the record of a call

The log warns that a voice server's snapshot repaired the record of a call, or that a voice
server no longer holds a call recorded on it.

**Means:** some of that voice server's reports never reached an API server. Either NATS was
unreachable or restarted, or a report kept failing (an `ERROR` line says which).

- The record is right again.
- Someone who appeared missing from, or stuck in, a call for up to a minute was this.
- Warnings that keep coming mean the voice server's link to NATS keeps dropping.

## A voice server was suspended

The log says `voice server suspended after failures from distinct users`, and the dashboard shows
it suspended.

**Means:** `failure_threshold` of this deployment's people failed to start a call on it within
`failure_window_seconds`. Each of them:

- was sent to it by a join offer moments before, and
- had not joined a call there lately.

A report from anyone else, a bot, or someone from another deployment does not count.

**What to do:**

- It takes calls again on its own after `failure_window_seconds`.
- Fix the cause meanwhile, usually its TLS proxy or its ports.
- Enabling it in the dashboard, or `voice-servers set <name> --enabled true`, ends the suspension
  at once.

The last server taking calls is never suspended. The log says `voice server left taking calls
despite failures` instead.
