# Ultra Lite

`aspen-ul` (`server/ul/`) is a whole deployment in one program on one machine, for a community that
needs no more than one server: it supervises the bundled PostgreSQL, holds the bus and the
short-lived store in its own memory, keeps the files in its data directory, and runs the voice
server inside itself. It shares every line of the API and voice servers' code with
`aspen-chat-server` and `voice_server`; only what they reach differs. It cannot be scaled, made
redundant, or highly available. A deployment that outgrows it moves to the full one.

## What runs

| Part                    | In Ultra Lite                                                                                                            | In the full deployment        |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------ | ----------------------------- |
| Database                | The bundled PostgreSQL, a child process, over a Unix socket in `run/` alone (`postgres.rs`)                              | PostgreSQL, wherever it is    |
| Bus                     | `aspen_bus::Bus::Memory`, in the process                                                                                 | NATS JetStream                |
| Short-lived store       | `aspen_ephemeral::Ephemeral::Memory`, in the process                                                                     | Valkey                        |
| Files                   | `MediaStore::Directory`, in `media/`, served at their own origin (`api::media`)                                          | S3 and its read path          |
| Voice server            | `voice_server::VoiceServer` in the process, its routes under `/voice-server` on the deployment's own port (`with_voice`) | `voice_server` processes      |
| API server and listener | `api::start_with` and `aspen_chat_server::serve`, as `aspen-chat-server` runs them                                       | `aspen-chat-server` processes |

The API server is given its configuration and backends (`app::context::Backends::InProcess`) rather
than reading `aspen.toml` and connecting: UL writes the settings it derives as TOML and lays
`aspen-ul.toml`'s `[server]` over them (`aspen_config::load_config_sources`), so they are checked as
a file's would be. `nats_url`, `nats_auth_token`, and `valkey_url`, which a server reaching those
services needs, are given addresses under `.invalid` that are never read.

The voice server reports over the same bus, at an inbox under its own prefix
(`voice_protocol::control::inbox_prefix`), so the API server answers its join-token key request
exactly as it answers one from a voice server elsewhere. It is registered as the voice server named
`aspen-ul`, made on first start and moved to the deployment's address on any start that finds it
elsewhere.

## The data directory

Everything the deployment keeps is in the directory `--data` names:

| Path            | What                                                                                          |
| --------------- | --------------------------------------------------------------------------------------------- |
| `aspen-ul.toml` | What the operator says (`config.rs`)                                                          |
| `postgres/18/`  | The cluster, made on first start with every locale category `C.UTF-8` of the builtin provider |
| `media/`        | The files (see [Directory storage](../../operators/configuration/media.md#mediadirectory))    |
| `logs/`         | PostgreSQL's log                                                                              |
| `run/`          | PostgreSQL's socket, readable by UL's user alone                                              |

The runtime (`--runtime`) holds the bundled PostgreSQL (`pgsql/`) and the web client
(`web-client/`).

## Sizes

What the deployment may use is `memory_mib`, or the machine's memory, or its cgroup's when less. It
sizes:

- the database pool: 6 connections below 2 GiB, 12 from it, since each grows to 13 to 17 MiB of the
  database's memory and keeps it (`postgres::Sizes`);
- PostgreSQL's `shared_buffers` (an eighth, from 64 to 512 MiB), `effective_cache_size`, and
  `maintenance_work_mem`, with `max_connections` the pool and ten more;
- what the bus's streams may reserve and what the short-lived store may hold, each a sixteenth, from
  16 to 256 MiB.

## Starting and stopping

UL refuses to run as root, as PostgreSQL does. It starts PostgreSQL (making its cluster and database
the first time), runs every migration not yet applied, starts the API server, registers and starts
the voice server, and listens. PostgreSQL is asked to stop at once should UL die without stopping it
(`PR_SET_PDEATHSIG`), and UL stops should PostgreSQL stop. Asked to stop, UL ends the servers and
the runtime's tasks first, then stops PostgreSQL, so nothing is left reaching a database that has
gone.

A restart empties the bus and the short-lived store. What correctness or security needs kept is in
PostgreSQL (`used_once`, `attempt_count`), and the bus's sequences start above any an earlier run
gave, so a client resuming from one reloads rather than resuming into the wrong place.

## Access and what clients hold

Ultra Lite adds no route and no record: what each reads and is told is decided by the same code as
in the full deployment, which `scripts/check_permissions.py` checks with `--media directory`. The
files' origin is never `public_url`'s, so a file a browser opens cannot act as the web client.
