# aspen-bench

`aspen-bench` answers one question: is this deployment up to the job its operator has in mind? It
plays a population of simulated users against a real deployment (signing in and starting up the
way the client does, holding the event stream open, chatting, tagging, reacting, replying in
threads, voting in polls, searching, posting pictures, reading history, reconnecting, and sitting
in voice calls with real SRTP media), measures what they experience, samples the servers' metrics
while it does, and writes a verdict against the service levels the profile names.

Run it against a staging deployment built like production. Running it against production works,
but the seeded population is visible to real users for the length of the run.

## A run, start to finish

```sh
cargo build --release -p aspen_bench -p aspen-chat-server

# 1. Plan the population. A profile is a TOML file or a built-in scenario's name.
target/release/aspen-bench plan smoke --run nightly1 --out plan.json

# 2. Seed it, on a machine with the deployment's aspen.toml (it needs the database).
target/release/aspen-chat-server bench seed --plan plan.json --out manifest.json

# 3. Run. Writes out/report.json and out/report.html; exits 0 on a pass, 1 on a fail, 2 on an error.
target/release/aspen-bench run smoke --manifest manifest.json --out out

# 4. Remove the population and everything that came to depend on it.
target/release/aspen-chat-server bench purge --run nightly1
```

Always use a release build of `aspen-bench`: a debug build cannot keep up with voice media and
reports loss that is its own.

`--api` and `--metrics` override the profile's `[target]`, so a built-in scenario can be pointed at
any deployment without copying it.

## Scenarios

`aspen-bench scenarios` lists the built-in profiles and `aspen-bench scenarios <name>` prints one,
as a starting point for your own.

| Scenario | What it asks |
|---|---|
| `smoke` | Does it work under light load? Run after every deployment change. |
| `small-group` | A dozen friends in one community, the smallest deployment Aspen serves. |
| `gaming-community` | A busy community evening: chat, voice, screen shares. |
| `public-community` | One very large community with many lurkers. |
| `announcement-fanout` | One message to a community of many thousands. |
| `reconnect-storm` | Everyone's connection drops at once; how long until they are back? |
| `bootstrap-storm` | Everyone opens the client at once. |
| `voice-evening` | Many concurrent calls with screen shares; media quality. |
| `mention-storm` | `@everyone` and tags in a large community: delivery, activity feeds, unread tags. |
| `search-heavy` | Searching deep history, within a community and everywhere. |
| `threads-and-polls` | Communities that talk in threads and decide by poll. |
| `soak` | Hours of steady load; does memory grow? |
| `chaos` | Parts of the deployment restart mid-run. |

## Profiles

A profile has these sections:

- `[target]` — `api`, the API origin, and `metrics`, the Prometheus endpoints to sample: the API
  servers', the voice servers', and any exporters (Postgres, NATS, Valkey, node) you run.
  `source_addresses` spreads users' connections over several local addresses: one address has
  only the system's ephemeral port range (about 28,000 ports on Linux) toward one server
  address, so a machine playing more than about ten thousand users needs several. The deployment
  also caps the connections one address holds before anyone signs in on them (`[connections]
  max_per_ip`, 512 by default); the suspension below lifts that cap for the generators'
  networks, so without it a reconnect storm of more users than that from one address is
  refused.
- `[population]` — how many users and communities to seed, the community size range (sizes fall
  off by rank, Zipf-like), channels per community, and history messages per channel. Of the
  history, `threads_per_channel` messages start a thread of `replies_per_thread` replies (one in
  fifty, and eight, by default) and `tagged_per_channel` tag a member (one in twenty); each
  channel also holds `polls_per_channel` polls (one), about half still open, with
  `votes_per_poll` votes each (a quarter of the community, at most a thousand). History is
  written in words drawn as real chat's are, a few common and many rare, in several scripts, so
  searches find much or little as real ones do.
- `[load]` — the share of users online, the ramp over which they connect, and the steady phase's
  length. Actions arrive as Poisson processes at the rates each behaviour names.
- `[behaviours.<name>]` — a share of the online users and their rates per hour: messages, DMs,
  tags of a member (`mentions_per_hour`), reactions, edits, deletes, history reads, reconnects,
  attachments, pictures (`images_per_hour`, JPEGs the deployment makes previews of), thread
  replies and threads started, polls opened and votes, searches, activity feed reads, saves and
  saved list reads, and presence changes (away, do not disturb, invisible, or back), with
  `[behaviours.<name>.voice]` for calls (calls per hour, minutes per call, screen sharing,
  bitrates). `viewing_share` is the share of these users with a channel open in front of them
  (all, by default): they name it to the server, see who is typing there, and report reading
  each message that arrives in it, as the client does. Before each message they write they type
  for `typing_seconds` (four), telling the channel.
- `[[events]]` — things that happen at a time into the run: `reconnect_storm` (a share of users drop
  at once), `spike` (rates multiply for a while), `announcement` (the owner of a community,
  `community` by index, largest first, tags `@everyone` in its first channel), and `command` (a
  shell command on the coordinator's machine, such as restarting a service).
- `[slo]` — the service levels that make the verdict: message delivery p50 and p99, request p99
  (overall and per route, `route_p99_ms`), connect p99, recovery p99 after a drop, error rate,
  voice loss, voice jitter, `connected_share`, the least share of the users meant to be online
  who must be connected through the steady phase (0.99 by default), and `metric_p99_ms` for any
  other measurement in the report, by name.
- `[limits]` — whether to suspend rate limits for the run (below).
- `[capacity]` — the online share to start at, step by, and stop at in capacity mode, and each
  step's length.

Latency is measured from when an action was due, not when the generator got to it, so a
generator that falls behind cannot hide a slow server; the generator's own lag is reported
beside it. Delivery latency is from a message's send to its arrival on every recipient's event
stream, with the clocks of the coordinator and its agents synchronised at the start. `delivery`
is channel messages'; DMs, thread replies, pictures, and announcements each have their own,
`delivery:dm`, `delivery:thread`, `delivery:image` (which includes the deployment making the
picture's preview, which the message waits for), and `delivery:everyone`.

## Rate limits

Hundreds of users from one address trip the per-address limits at once. A profile with
`[limits] suspend = true` suspends them for the run through NATS (`nats_url`, `nats_token`) and
resumes them at the end, unless someone else's suspension has replaced it by then:

```toml
[limits]
suspend = true
scope = "networks"            # the default; "all" lifts every limit for everyone
networks = ["203.0.113.0/28"] # the generators' addresses
nats_url = "nats://nats.staging:4222"
nats_token = "…"
```

With `scope = "networks"` only requests and connections from those networks skip the limits that
count by address; every per-user and global limit stays, so the run still measures a deployment
with its limits on.

The same suspension is available by hand:

```sh
aspen-chat-server limits suspend --for 2h --network 203.0.113.0/28 --reason "load test"
aspen-chat-server limits status
aspen-chat-server limits resume
```

A suspension always ends by itself: at `--for`, and never later than `max_suspension_seconds`
after it began (a day by default, set in each server's rate limit configuration), so a forgotten
one cannot leave a deployment open. Every server logs a warning each minute while one is in force.
There is deliberately no HTTP endpoint for this.

## Several machines

One machine runs out of sockets, CPU, or bandwidth long before a large deployment does. Start the
coordinator waiting for agents, then start the agents on other machines:

```sh
aspen-bench run public-community --manifest manifest.json --out out --agents 4 --listen 0.0.0.0:7700
aspen-bench agent --coordinator ws://coordinator:7700   # on each of the four
```

The coordinator deals users to agents, starts everyone at the same moment, and merges their
measurements. Each agent reports its generator lag; if it is high, add agents.

## Capacity

`--mode capacity` repeats the steady phase with more users online each step (`[capacity]`) and
reports the largest step at which every service level held, with the first one that broke and
what the servers' metrics say ran out. Interrupting it (Ctrl-C, or `SIGINT` from a guard that
watches the generator's memory) ends the step under way and reports the steps that finished.

## Reading a report

`report.html` is self-contained. It shows the verdict and each service level, latency percentiles
by phase and over time, per-route request latency, voice loss, jitter, and round-trip time,
counters, what any `[[events]]` commands printed, and findings from the sampled metrics:

- process CPU near the host's core count;
- database pool connections waiting, sustained;
- NATS publish or Valkey rate-limit check latency high;
- rate limit refusals (the run was throttled, so its latencies are not the server's);
- a mediasoup worker near one full core;
- memory growth per hour, fitted over a steady phase of at least ten minutes: resident memory,
  and the heap the servers' allocator reports (`aspen_memory_allocated_bytes`). Resident memory
  that grows while the heap does not is the allocator keeping freed memory; a growing heap is a
  leak.

`aspen-bench html report.json --out report.html` renders a report again.

## Regression checks in CI

```sh
bench/scripts/regression.sh baseline.json smoke 0.1
```

plans, seeds, runs, and purges (on any exit), and fails when the scenario misses a service level
or any steady-phase p99 is more than 10% slower than the baseline's. `aspen-bench compare
old.json new.json` does the comparison alone. Keep the baseline from the same hardware.
