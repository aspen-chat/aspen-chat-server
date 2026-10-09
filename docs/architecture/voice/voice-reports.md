# Voice reports

Voice servers tell the API servers what happens in their calls by publishing `VoiceReport`s. The API servers' record of calls follows these reports. How lost reports are repaired is in [Snapshots](snapshots.md); how a report becomes rows and events is in [Sessions](sessions.md).

## Where it lives

| Piece | Code |
|---|---|
| Report types and subjects | `voice_protocol::control` (`VoiceReport::subject`, `subject_server`, `REPORT_PARTITIONS`) |
| Reading the stream | `app::voice::reports` |
| Checking where a report came from | `reported_by` |
| Checking a new call or participant was offered | `servers::offered_for` |
| Applying a report | `app::voice::sessions::apply_report` |
| Sending, on the voice server | `Reporter` |
| Where each session is, for speaking changes | `voice_session_homes` |

## The stream

- JetStream stream: `aspen_voice_reports`. The API servers create it.
- Storage: in memory, kept until applied (work-queue retention), a day at most.

### Lanes

Reports are spread over `REPORT_PARTITIONS` (64) lanes:

- A report about a call goes by its channel: the last byte of the channel's id modulo 64. That byte is random in a UUIDv7. Every report about a call names its channel.
- A report about the whole server (its load) goes by the server's id.
- Speaking changes go in 64 lanes of their own.

| Kind | Subject |
|---|---|
| Everything but speaking | `aspen.voice.report.{lane}.{server}` |
| Speaking changes | `aspen.voice.speaking.{lane}.{server}` |

### Reading

- Every lane has one durable consumer, shared by all API servers.
- The consumer hands out one report at a time, and the next only once that one is acknowledged.
- Every API server reads every lane.
- Each API server applies at most half as many reports at once as its pool has database connections. Requests always have the other half.

So the reports about one channel are applied once, in the order they were sent, by whichever API server takes each. A channel's calls are applied one after another (a channel's call always ends before its next one starts). The 128 lanes are applied side by side.

On the voice server, reports go out through one queue (`Reporter`) in the order they are made.

## Where a report came from

The source of a report is the server its subject names (`subject_server`), never what the report says. `reported_by` drops a report unless:

- it is on the very subject that server would send it on;
- it names that server, if it names one;
- if it is about a recorded session: that session is recorded on that server and in the channel the report names;
- if it is about a file (an offer needs one): the channel's recorded call, if any, is on that server.

### Only where a server was sent

A call or a participant not yet recorded is recorded only where the API server sent someone.

1. Each join offer notes in Valkey that each candidate was offered for that channel and user, for the token's lifetime and ten minutes more:
   - `voice_offered_for:{server}:{channel}:{user}`
   - `voice_offered_for:{server}:{channel}` (without the user)
2. A report of a new call (`sessionStarted`, or a snapshot's) needs the channel's note.
3. A report of someone joining (`participantJoined`, or a snapshot's) needs the user's note (`servers::offered_for`).

A voice server taken over cannot make up a call, or someone in one, anywhere it was not sent. A Valkey outage believes the report, so calls go on through it.

## NATS users

Each voice server may sign in to NATS as a user of its own (`[nats_user]` in `voice_server.toml`). The API servers then sign in as theirs (`[nats_user]` in `aspen.toml`). A voice server's user may only:

- publish on its own report subjects,
- read its own commands and the rate limit suspension,
- receive replies under its own inbox prefix (`voice_protocol::control::inbox_prefix`, `_INBOX_voice.{server}`).

A voice server taken over can then misreport its own calls and no one else's. [Installing](../../operators/installing/6-voice-servers.md#give-it-a-nats-user) lists the permissions.

### What a voice server taken over can still cause

The permissions bound what a voice server can say, not everything it can cause. NATS checks a request's reply subject against no one's permissions. So a voice server can name any subject as the reply to a request it may send, and whoever answers publishes there.

- The API servers answer a voice server only on its own inbox (`is_voice_inbox`).
- The JetStream API answers the requests the rate limit suspension needs (`$JS.API.INFO`, the bucket's reads) wherever it is told.

So a voice server taken over can still have messages it did not write published on the API servers' subjects:

| Subject | Effect of each such message |
|---|---|
| `aspen.plugins.changed` | Every API server reloads every plugin. |
| The wake-up subjects of held messages, previews, and the outbox | A pass. |
| A key of `aspen_rate_limits` or `aspen_settings` | Overwrites the value with one that is not read. This ends an operator's suspension of rate limits. |

It cannot make the JetStream API act, since such a message carries no reply subject. Nothing it can have published this way is believed as a report, a command, or an event. Only a NATS account of the voice servers' own, importing from the API servers' account just the subjects they need, would keep replies within it.

## Speaking changes

A speaking change writes nothing. Once its session is found recorded on its server, it is passed on as a `voiceSpeaking` event to the channel it names, without a transaction.

A session's server and channel never change, so where each session is is remembered for ten seconds (`voice_session_homes`, a `Recent`). A lively call's speaking reads the database once per call every ten seconds rather than once per change.

## Acknowledgement and retries

- A report is acknowledged once it has been applied.
- One that fails because the database or NATS is unreachable is tried again a second later, up to three times. Its lane's later reports are held back meanwhile.
- Any other failure drops it.
- A report whose acknowledgement was lost is delivered again. Applying a report twice changes nothing more.
- Removing a voice server clears its waiting reports.

## Load reports

- Voice servers report their load every fifteen seconds.
- A load report is applied outside a transaction, since it writes one row.
- One from an id no registered server has updates nothing. It is noted for the dashboard instead (see [Administration](../administration/index.md)).
- `load` records when the stream took the report, not when it was applied. A report that waited in the stream does not make a dead server look alive.

## Metrics

Each API server counts, by report type:

| Metric | Measures |
|---|---|
| `aspen_voice_reports_applied_total` | Reports applied |
| `aspen_voice_report_wait_duration_seconds` | Time from reaching the stream to being applied |

The wait shows the record of calls falling behind.
