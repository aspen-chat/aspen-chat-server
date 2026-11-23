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
| Files                   | `MediaStore::Directory`, in `media/`, served at their own origin (`api::media`), or S3 when `[server.media.s3]` gives it | S3 and its read path          |
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
| `backups/`      | Dumps made before applying migrations, the newest three, and a backup's staging while it runs |
| `acme/`         | The ACME account, and the certificate and its key, readable by UL's user alone                |
| `logs/`         | PostgreSQL's log                                                                              |
| `run/`          | PostgreSQL's socket and the control socket, readable by UL's user alone                       |
| `runtime/`      | The runtime unpacked from the binary (see below)                                              |

## The runtime

What UL runs beside its own code, the bundled PostgreSQL (`pgsql/`) and the web client
(`web-client/`), is a zstd-compressed tar appended to the binary by `scripts/package_ul.py`
(`bundle.rs`), so `cargo build` needs neither pnpm nor a PostgreSQL build. The binary ends with a
trailer: the archive's SHA-256, its length, and a magic number. On start, UL unpacks the archive
into `runtime/<hash>/` unless that directory is already there, unpacking beside it, checking the
hash, and only then renaming it into place, so a directory by that name always holds the whole
runtime. A damaged binary is refused. `runtime/current` names the runtime started last; when a new
binary replaces it, the one it named is kept beside the new one, for a PostgreSQL major-version
upgrade to run its binaries, and every older one is removed. `--runtime` names a directory to use
instead, for development.

PostgreSQL is built from source against the Debian 12 sysroot `scripts/baseline_build.py` uses,
without OpenSSL, ICU, or readline, with zlib linked in statically, the extensions the migrations
create (`pg_trgm`, `btree_gin`), and only the programs that running, dumping, restoring, and
`pg_upgrade` need. It needs no library but the C runtime and its own `libpq`, which its programs
find through their run path. `aspen-ul` itself is built by `baseline_build.py` like the other
servers, and needs `libssl3`.

## TLS

`[tls] mode` is `off` (plain HTTP, for a proxy on the machine or a deployment at `localhost`),
`files` (a certificate and key the operator keeps), or `acme` (`acme.rs`). With `acme`, UL asks an
ACME authority, Let's Encrypt unless `directory` names another, for one certificate naming
`public_url`'s host and, when it serves its files, `media_url`'s. It answers the authority's
TLS-ALPN-01 validation on the port it serves at (`acme-tls/1` among its ALPN protocols, a
`ResolvesServerCert` answering a validation with its challenge certificate and anything else with
the live certificate), so no other port needs to reach the machine and nothing else stops serving
while it does. Until the first certificate arrives, handshakes are refused. The certificate is
renewed two thirds through its lifetime, and at once when it does not name every name, looked at at
least every twelve hours; a failed issuance is retried after a minute, doubling to six hours, since
authorities limit failed validations.

With TLS on, `redirect_port` (80, or 0 for none) answers plain HTTP with a permanent redirect to the
same path at `public_url`, or at `media_url` for a request naming its host (`redirect.rs`); the
redirect is to the deployment's own address whatever the `Host` says.

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

## Installing

`aspen-ul install`, as root (`setup.rs`), makes the system user `aspen-ul`, its data directory
(`/var/lib/aspen-ul`, mode 0750), copies the binary to `/usr/local/bin/aspen-ul`, and writes a
systemd unit that runs it as that user, with `CAP_NET_BIND_SERVICE` alone (ports 80 and 443) and the
filesystem read-only but for the data directory. The unit stops UL with SIGTERM to UL alone
(`KillMode=mixed`), which stops PostgreSQL itself, and restarts it when it fails; a PostgreSQL that
stops makes UL stop, so the unit restarts both. Run again, `install` replaces the binary and
restarts the deployment, which is how it is upgraded.

Started as root for anything else, UL becomes the user that owns the data directory before any
thread starts, so `sudo aspen-ul <command>` reads and writes as the deployment does; it refuses a
data directory root owns.

`aspen-ul init --domain` writes `aspen-ul.toml`, asking for the domain again (or `--yes`), since it
can never change. `public_url` is always `https://` and the domain alone: people reach the
deployment at 443, whether through a router forwarding it or, with `--tls off`, a proxy on the
machine, which then reaches UL at `--port` on loopback. With `--invite-only` it makes creating an
account take an invite and makes one for the first administrator, printing its `/register?invite=`
link.

## Backups

`aspen-ul backup DIR` writes the whole deployment into a directory of its own, mode 0700 since the
dump holds the deployment's secrets, while it runs (`backup.rs`): `database.dump`
(`pg_dump --format=custom --no-owner --no-privileges`), `media/`, `aspen-ul.toml`, `acme/`, and
`backup.json`, written last. Files kept in S3 are not in it.

The files and the database are taken at one moment. The command opens a repeatable-read transaction
and exports its snapshot, then takes `media/removals.lock` exclusively, which every removal of a
file in directory storage (`delete`, `move_object`) holds shared while it removes
(`media_store::directory::REMOVALS_LOCK`), hard-links every object and record into
`backups/.staging-<pid>/`, and lets go. Only then does `pg_dump --snapshot` dump that snapshot, and
the staged files move into the backup, copied if it is on another filesystem. So no file the
snapshot names can be removed before it is linked, and removals wait only as long as linking takes.
A file written after the snapshot may be among them, one no row names. Files are only ever written
to a scratch name and renamed, so a link never sees a file change.

`aspen-ul restore DIR` refuses while the deployment runs. It moves the data directory's
`aspen-ul.toml`, `postgres/`, `media/`, and `acme/` into `replaced-<time>/`, copies the backup's in,
makes a new cluster, and restores the dump into it in one transaction. Sessions, the federation key,
and every secret are in the database, so they come back with it.

Before a start applies migrations to a database that already has some, UL dumps it to
`backups/before-migrating-<time>.dump`, keeping the newest three, so a failed upgrade can be undone
with the binary before it.

## Operator commands

`aspen-ul <command>` runs any of `aspen-chat-server`'s operator commands against the deployment, in
its own process, with its answer on standard output and its logs on standard error. A command
changes the database directly, as it does in the full deployment, and publishes what it changes on
the bus, which here is the running server's memory: so the command reaches it through the control
socket the server serves, `run/control.sock`, readable by UL's user alone (`aspen_bus::remote`).
Over the socket a command publishes, sizes and purges streams, and reads and writes buckets; what
only a server does (subscribing, reading streams through consumers) is refused there. Every place a
command reaches the bus asks `AspenConfig::connect_bus`, which answers the process's own bus
(`use_process_bus`) when it has one.

With the deployment stopped, a command starts PostgreSQL itself and stops it after, and what it
publishes reaches no one, since no one is connected to hear it. A server starting refuses a data
directory another one serves, which it tells by its socket answering, and replaces a socket a
stopped one left.

## Access and what clients hold

Ultra Lite adds no route and no record: what each reads and is told is decided by the same code as
in the full deployment, which `scripts/check_permissions.py` checks with `--media directory`. The
files' origin is never `public_url`'s, so a file a browser opens cannot act as the web client.
