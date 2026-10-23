# NATS memory

NATS keeps two of the deployment's streams in memory: the events of the last minute
([Event routing](event-routing/index.md)) and the voice servers' reports not yet applied
([Voice](voice/index.md)). This page covers how large each is, how that follows NATS's memory, and
what the dashboard shows of it. Why it works this way is in the
[design notes](nats-memory-design-notes.md).

NATS counts a memory stream's `max_bytes` against its memory limit when the stream is made or
resized, whatever the stream then holds, and refuses one that does not fit (JetStream error 10028).
So a size is a reservation, and it has to fit.

## Where it lives

| Part                                    | Code                                                                                                         |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Sizes, fitting, the dashboard's figures | `app::nats_memory` (`sizes`, `fit`, `fit_locked`, `usage`, `Monitor`)                                        |
| The streams' configuration              | `app::event_feed::stream_config`, `app::voice::report_stream_config`                                         |
| The settings                            | `event_stream_mib`, `voice_report_stream_mib` ([Deployment settings](administration/deployment-settings.md)) |
| The monitoring port                     | `[nats] monitor_url` in `aspen.toml`                                                                         |
| The recurring job                       | `fitStreams` ([Kinds](jobs/kinds.md#fitstreams))                                                             |
| The dashboard                           | `natsMemory` in `GET /admin/fleet` ([Dashboard](administration/dashboard.md#nats-memory))                    |
| Check                                   | `scripts/check_nats_memory.py`                                                                               |

## What NATS has

`fit` reads, each time it runs:

- **The account** (`$JS.API.INFO`): the memory its streams hold and have reserved, and its limit
  when NATS gives the account one.
- **The server**, when `[nats] monitor_url` names its monitoring port: `max_memory_store`, and what
  every stream on it holds and has reserved (`/jsz`). A NATS server says its own limit nowhere else.
  An `https` address is read with `[nats.tls]`'s authorities and certificate.
- **The two streams**, for what they have reserved now.

What is available to the two streams is the limit less what every other stream has reserved. With
both limits known it is the lesser of what each leaves. With neither it is unknown.

## Sizes

`sizes` gives each stream its size, in whole MiB.

| Events    | Voice reports | Available known                                                   | Available unknown       |
| --------- | ------------- | ----------------------------------------------------------------- | ----------------------- |
| Automatic | Automatic     | Voice reports take an eighth, at most 1 GiB. Events take the rest | 1 GiB and 128 MiB       |
| Set       | Automatic     | Voice reports take what is left, at most 1 GiB                    | The setting and 128 MiB |
| Automatic | Set           | Events take what is left                                          | 1 GiB and the setting   |
| Set       | Set           | The settings                                                      | The settings            |

- A setting is at least `MIN_STREAM_MIB` (16), and at most what `INTEGER` holds.
- Settings that come to more than is available are refused (`streamSizesOverLimit`), as is a change
  that leaves an automatic stream under 16 MiB (`streamMemoryTooLittle`).
- Where the limit is unknown, NATS's own refusal is passed on (`streamSizesRefused`).
- **While a monitoring port is named and does not answer**, an automatic stream keeps the size it
  has, and is not shrunk to the fallback.

## When the streams are fitted

| When                   | How                                                                                                                                                                   |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A server starts        | `fit_locked` in `GlobalServerContext::new`, with `always`, so the rest of each stream's configuration is this version's. A server that cannot fit them does not start |
| A size setting changes | `fit` inside the change's transaction, after the row is written and before its doorbell, so a size NATS refuses rolls the change back                                 |
| Every fifteen seconds  | The recurring job `fitStreams`                                                                                                                                        |

- All three hold the settings row's lock (`deployment_settings::load_locked`), so none applies sizes
  another has since changed.
- A stream that shrinks is resized before one that grows, so the room it gives up is there to take.
- A stream already the right size is left alone, except at start.
- `fitStreams` is what makes the streams again after NATS restarts without them, and what follows a
  `max_memory_store` NATS was restarted with. NATS does not take a new `max_memory_store` on a
  configuration reload.
- A size change that fitted one stream and then failed leaves the streams unlike the row until the
  next `fitStreams`.

## What a smaller stream loses

A stream made smaller than what it holds lets its oldest messages go at once (`DiscardPolicy::Old`),
as one that fills does.

- **Events:** an API server whose feed had not read them sees the sequence jump, drops every local
  connection, and each client rebuilds its state from REST
  ([The event feed](event-routing/index.md)). A client resuming from before the oldest event kept
  reloads likewise.
- **Voice reports:** the voice server's next snapshot repairs whatever was let go.

## Who can observe it

| What               | Who                                   | Route                                                |
| ------------------ | ------------------------------------- | ---------------------------------------------------- |
| The settings       | Holders of Manage deployment settings | `GET`, `PATCH /admin/settings`, `settings show\|set` |
| The memory figures | Holders of View dashboard             | `natsMemory` in `GET /admin/fleet`                   |

- No event carries either. The dashboard reads the fleet every ten seconds while it is open.
- Losing the permission ends the next read, as for the rest of the fleet and the settings.
- `natsMemory` is `null` when NATS did not answer, and the rest of the fleet is still shown.
