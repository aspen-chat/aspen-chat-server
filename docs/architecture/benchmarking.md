# Benchmarking

`bench/` (`aspen-bench`) says whether a deployment is up to a job described by a profile: population, load, behaviours, timed events, and service levels. `bench/README.md` is the operator guide. Design notes: [benchmarking-design-notes.md](benchmarking-design-notes.md).

| Part | Where |
| --- | --- |
| Benchmark tool | `bench/` (`aspen-bench`) |
| Built-in profiles | `bench/profiles/` |
| Shared types | `bench_protocol/` (`aspen_bench_protocol`): the plan, the manifest seeding writes, and the coordinator-agent messages |
| Seeding | `app::benchmark::seed` |
| Rate limit suspension | `aspen_limits::suspension`, NATS KV bucket `aspen_rate_limits` |
| Metrics | `metrics/` (`aspen_metrics`), whose constants name every series |

## A run

A run is plan, seed, run, purge.

### 1. Plan

`aspen-bench plan <profile> --run <tag>` writes an `aspen_bench_protocol::SeedPlan`.

### 2. Seed

`aspen-chat-server bench seed` writes the population straight into the database (`app::benchmark::seed`).

- Writes are batched, and history is backdated with UUIDv7 ids.
- History is written as posting would write it: some messages tag a member (with their `mention` rows), some start threads (the thread channel, its replies, its counts, and its followers), and polls sit among it, some closed (with the message the poll closer posts) and some open, with votes. Its words come from `aspen_bench_protocol::words`, drawn by Zipf's law, which the tool searches for too.
- The manifest names each community's seeded threads and open polls, for the run to reply in and vote in.
- Every user and community it made is recorded against the run in `benchmark_run`, `benchmark_user`, and `benchmark_community`.
- Benchmark users are ordinary users named `bench-<run>-<n>` with one shared password.
- The password is the plan's, or else one drawn at random for the run. `bench seed` prints it and the manifest carries it. The run's record in `benchmark_run` leaves it out.

### 3. Run

`aspen-bench run` plays the users over the real API and event stream, the way the client does.

- Users start up as the client does, with its requests and the records it sideloads, and the client's own traffic goes on beside their actions: presence polls, activity, and, for those with a channel open, the `viewing` frame, typing, and read position reports.
- Actions are open-model Poisson, with latency measured from when each action was due.
- Delivery is measured per kind of message: channel messages, DMs, thread replies, pictures (held for their previews), and `@everyone` announcements.
- A coordinator can spread users over agents on other machines (`aspen-bench agent`). Clocks are synchronised by Cristian's algorithm, so delivery latency is measured across machines.
- Calls use real SRTP through `produceRtp` and `consumeRtp`, measuring loss, RFC 3550 jitter, and round-trip time.
- It samples the `[target] metrics` endpoints and names what ran short in the report (`report.json`, `report.html`).
- `--mode capacity` raises the load step by step until a service level breaks.

| Exit code | Meaning |
| --- | --- |
| 0 | Pass |
| 1 | Fail |
| 2 | Error |

`aspen-bench compare` and `bench/scripts/regression.sh` fail on a slower p99, for CI.

### 4. Purge

`aspen-chat-server bench purge --run <tag>` deletes the run's users and communities and every row that came to depend on them, however the run went.

1. It reads the foreign key graph from `pg_constraint`.
2. It marks the doomed rows in temporary tables.
3. It clears the nullable references among them that close a cycle. Any other is left to the order of deletion, since clearing it could break a check on its table.
4. It deletes children first in one transaction.
5. It removes stored media after the transaction commits.

When changing the schema:

- A table added with a foreign key to anything a benchmark user can make is covered without changes.
- **A new stored-media column must be added to the purge's list of objects.**

## Suspending rate limits

Rate limits would stop hundreds of users sharing an address, so an operator can suspend them:

```
aspen-chat-server limits suspend --for 2h [--scope networks --network CIDR…|--scope all] --reason …
```

with `resume` and `status`.

| Scope | Effect |
| --- | --- |
| `networks` | Exempts those networks from the limits that count by address (the API listener's `[connections]` address and network caps and the voice server's pending socket cap included), keeping every other limit |
| `all` | Lifts them all |

- The suspension is one record in the NATS KV bucket `aspen_rate_limits` (`aspen_limits::suspension`), watched by every API and voice server.
- It always ends by itself: at its `until`, and never later than each server's own `max_suspension_seconds` after it began.
- Servers log a warning every minute while one is in force.
- It is a command run with the deployment's credentials, never an HTTP endpoint.
- A profile with `[limits] suspend = true` suspends for its run and resumes afterwards if the record is still its own.

## Metrics

Both servers export Prometheus metrics on a loopback listener. A new metric gets its name as a constant in `aspen_metrics`.

### API server

- Requests by route and status, with latency. A route is the method and path template; a method that is not a standard HTTP one is named `OTHER`, so the series stay bounded.
- Database pool connections.
- Event publish latency, event streams, and deliveries.
- The event feed's retained window, routing time, and dropped streams.
- Rate limit checks and refusals.
- Voice reports applied, and how long each waited between reaching the report stream and being applied, by type.
- Mail sent and given up: `aspen_emails_sent_total`, `aspen_emails_failed_total`.
- Attachment previews made, given up, and how long each took: `aspen_attachment_previews_made_total`, `aspen_attachment_previews_failed_total`, `aspen_attachment_preview_duration_seconds`.
- Pushes by outcome and those queued: `aspen_pushes_total`, `aspen_pushes_waiting`.
- Process CPU and memory.

### Voice server

Configured by `[metrics]` in `voice_server.toml` (`127.0.0.1:9465`).

- Rooms, participants, producers, consumers, and transports.
- Signalling frames and refusals.
- mediasoup worker CPU.

### Both

jemalloc's own figures: `aspen_memory_allocated_bytes`, `_active_`, `_resident_`, `_mapped_`, `_retained_`.

| Pattern | Means |
| --- | --- |
| Allocated keeps rising | A leak |
| Resident well above allocated | Memory the allocator keeps |

The bench reports heap growth beside resident growth for that reason.
