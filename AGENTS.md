Aspen is a free and open source federated community chat solution implemented to the highest code quality standards. Aspen takes a lot of inspiration from the Discord chat platform, but aims
to reach an even higher degree of quality and user delight. Aspen clients will be made for iOS, Android, Windows, Mac, and Linux.

Aspen seeks to support community text chat, voice calls, and video feeds from cameras, screen sharing, and videogame capture via libobs.

Aspen rigidly adheres to DRY, and is split into three layers in implementation. This repo contains the server-side implementation in `server/` and the cross-platform client in `client/` (see `client/AGENTS.md`).
Once complete, Aspen will have rigorous test suites, hardware benchmarking programs, and infrastructure as code to aid with deploying its many docker containers.

Aspen aims to be horizontally scalable, providing a user experience suitable for millions of users, and also to be just as suitable for communities containing less than a dozen users.

Federation, letting a user of one deployment use others, is built in five phases, all in place: each deployment's identity, key, policy, and directory of other deployments; signing in abroad; a client holding sessions on several deployments; DMs across deployments; and revocation, deletion notices, and moderating foreign users. See `docs/architecture/federation.md`. Extend it only with explicit direction.

## Tech Stack

- **Web framework:** Axum (with `utoipa-axum` for OpenAPI integration)
- **Async runtime:** Tokio (multi-threaded)
- **ORM:** Diesel with `diesel-async` and `deadpool` connection pooling
- **Database:** PostgreSQL
- **Message broker:** NATS JetStream (via `async-nats`)
- **Password hashing:** Argon2
- **Passkeys:** `webauthn-rs`, which links the system OpenSSL
- **ID generation:** UUID v7
- **Serialization:** Serde + `serde_json`
- **OpenAPI generation:** utoipa
- **JSON Schema generation:** schemars
- **TLS:** rustls (with self-signed cert generation via `rcgen` for development)
- **Federation signatures:** `ring` (Ed25519 keys, compact JWS with EdDSA)
- **Email:** `lettre` over SMTP, written from `askama` templates (`server/templates/email/`), with `chrono-tz` for each digest's time zone; see Email
- **Attachment previews:** `image` (decoders written in Rust), `fast_image_resize`, `moxcms` (colour profiles), and `webp` (libwebp, built from source) for pictures, and the operator's `ffmpeg` and `ffprobe`, run as processes of their own, for videos' posters; see Attachment previews
- **File transfers:** WebRTC data channels between clients, with STUN and a TURN relay in the voice server (`turn`, from webrtc-rs)
- **Plugins:** `wasmtime` running WebAssembly components (`spec/plugin.wit`), with `wasmtime-wasi` giving a component's standard library empty system interfaces; see Plugins
- **Metrics:** `metrics` with the Prometheus exporter (`aspen_metrics`)
- **Allocator:** jemalloc (`tikv-jemallocator`) in both servers, replacing `malloc` for the whole process so C and C++ libraries (mediasoup's worker) use it too, with its background thread on (`malloc_conf` in each `main.rs`) so freed memory returns to the system when a server goes idle; its statistics are exported as `aspen_memory_*`

## Building and Running

`docs/operators/` is the guide for people who run a deployment: installing, every setting, federation, backups, and troubleshooting by problem code. It describes the code as it stands, like every comment: a change to a setting (`aspen_config.rs`, `voice_server/src/config.rs`), an operator command, a `ProblemCode`, or what an operator must open, route, or back up updates it in the same commit.

1. Start infrastructure services:
   ```
   docker-compose up -d
   ```
   This starts PostgreSQL (port 5432) and NATS with JetStream (port 4222).

2. Run database migrations:
   ```
   cargo run -p aspen-migrate -- up
   ```
   The connection string is resolved from `--database-url`, then `DATABASE_URL`, then `database_url` in `aspen.toml` (same file the server reads).
   After any schema-changing migration, regenerate `server/src/database/schema.rs`:
   ```
   cd server && diesel print-schema > src/database/schema.rs
   ```
   If you are upgrading a database that was previously managed by Diesel's migration runner, run `cargo run -p aspen-migrate -- import-diesel` once to copy `__diesel_schema_migrations` history into `__aspen_migrations` (mapping each Diesel `version` to the registry ID with the matching `YYYYMMDDHHMMSS` prefix, preserving `run_on`) and drop the Diesel bookkeeping table. The command refuses to run if `__aspen_migrations` is already populated or if any Diesel row can't be mapped to a registry entry.

3. Build the web client and run the server:
   ```
   (cd client && pnpm install && pnpm build)
   cargo run
   ```
   A deployment is one origin: each API server serves the web client too (`api::web_client`), and refuses to start without a built one. The server reads configuration from `aspen.toml` and from environment variables with the `ASPEN_` prefix, which override the file (`__` separates nested keys: `ASPEN_VOICE__IDLE_SESSION_SECONDS`). Key config values:
   - `public_url` — the deployment's one address (`https://chat.example.org`, an origin alone), its API and web client together; required. Every link to the deployment is built on it (`GET /deployment` gives it to clients as `webClientUrl`), and what else names the deployment follows from it: passkeys belong to its host (offered over `https`, or at `localhost` and names under it), and over `https` its host, with `:port` when not 443, is the federation domain, which never changes, since other deployments pin the key they find there: the first server to start with it pins it in the database, and a server started with another refuses to start. An `http` address takes no part in federation. In development it is the Vite dev server's, `http://localhost:5173`, which proxies the server's own paths to it
   - `[web_client] dir` — the built web client (`client/packages/app/dist` by default), which this server serves at `public_url`, answering every path the API does not own with its `index.html` and Open Graph tags (see The web client)
   - `database_url` — PostgreSQL connection string
   - `database_pool_size` — the most database connections the server holds (two per logical CPU by default); every write holds one until its event is acknowledged
   - `database_pool_wait_seconds` — how long a request or task waits for a database connection before it is refused with `serverBusy` (ten by default). Do not hold one connection while waiting for another (give it back first, as `app::message` does around plugins): when every connection is held that way the pool has none to give, and every waiter is refused
   - `nats_url` — NATS server address
   - `nats_auth_token` — NATS authentication token, or `[nats]` `user` and `password` when NATS has users, as it does once each voice server signs in as a user of its own allowed only its own subjects (see Voice)
   - `[voice]` — `token_secret` shared with the voice servers, the failure threshold and window, the join token lifetime, the candidate cap, the two silence limits, and the idle call limit. The voice servers themselves are rows of `voice_server`, registered from the dashboard or with `aspen-chat-server voice-servers add` (see Voice)
   - `[media.s3]` — the object storage for attachments, icons, and preview images: `endpoint` (the S3 API as this server reaches it), `public_endpoint` (the same API as clients reach it, which the presigned upload URLs they are handed name; left out, they name `endpoint`, which only suits clients on this machine), `public_base_url` (where clients download objects), `bucket`, `region`, the credentials, and `upload_url_ttl_seconds`. Clients upload straight to storage, so `public_endpoint` must be reachable from every client and allow their origins (`public_url`'s, and the desktop and mobile apps') by CORS.
   - `[media] max_attachment_bytes` — the largest attachment anyone may upload (256 MiB by default), which upload URLs are signed for; files a browser could run are stored and served as `application/octet-stream` (`app::attachment::INLINE_TYPES`)
   - `[media.previews]` — `make` (whether this server makes previews; every server queues them), `concurrency`, `max_picture_bytes` and `max_picture_pixels`, `ffmpeg` and `ffprobe` (the programs that take videos' posters; empty, none are taken), `max_video_bytes`, and `ffmpeg_memory_mib` (the address space each run of them may map); see Attachment previews
   - `[limits] max_communities_per_user` — the most communities one user may belong to (500 by default); it bounds what an event stream connection reads and how far a profile change fans out. `max_event_streams_per_user` (20) and `max_event_streams_per_address` (200) cap the event streams one user and one client address hold open on each API server
   - `[auth]` — `reverify_seconds` (how recent a verification security changes need, ten minutes by default), `password_hashing_threads` (one per logical CPU by default) and `password_hashing_wait_seconds` (ten) bound password work; see Sign-in and security.
   - `[presence] away_after_seconds` — how long a connected user may go without using Aspen before showing as away (600 by default)
   - `[rate_limits]` — per-endpoint rate limits laid over the built-in ones in `server/src/rate_limits.toml`, which documents the format; see Rate limits below. `max_suspension_seconds` (a day) caps how long an operator's suspension of them lasts on this server (see Benchmarking)
   - `[metrics]` — `enabled` and `listen_addr` (`127.0.0.1:9464`) for the Prometheus endpoint, a listener of its own; keep it off public interfaces
   - `[connections]` — `max` and `max_per_ip` (100000 and 512), the connections the listener holds open at once, of everyone and of one address (trusted proxies count only toward `max`), and `handshake_seconds` and `header_read_seconds` (10 and 30), how long a client has for its TLS handshake and an HTTP/1.1 request's headers (`connections`)
   - `event_queue_size` — how many events one event stream connection may have waiting to be written (512 by default); a connection that falls that far behind is dropped and resumes
   - `event_feed_shards` — how many tasks route events to this server's event stream connections (one per logical CPU by default; see Event routing)
   - `[federation]` — `standing_interval_seconds` and `standing_grace_seconds` (how often foreign users are confirmed with their homes, and how long an unreached home is tolerated), and `[federation.development]` (`extra_root_certificates`, `allow_private_addresses`) for deployments side by side on one machine; see Federation
   - `[push] enabled` — whether apps may ask to be woken and messages wake them (true by default); see Push
   - `[email]` — `smtp_url`, `from`, `send` (whether this server sends and makes digests; off, it only queues and needs no `smtp_url`), and `max_per_second` (the deployment-wide sending rate, counted in Valkey). Without it the deployment sends no mail and the email deployment settings cannot be turned on; the `mailpit` service in `docker-compose.yaml` catches a development server's mail (`smtp_url = "smtp://localhost:1025"`, inbox at http://localhost:8025). See Email
   - `[plugins]` — `intercept_millis` (25), `observe_millis` (ten seconds), and `route_millis` (three seconds), how long a plugin's call may take to decide a message, handle an event, and answer a route, `memory_mib` (64), the most one call may use, and `concurrency` (two per logical CPU) and `concurrency_per_plugin` (one per logical CPU), how many calls run at once in all and of one plugin; which plugins are installed, and their settings, are in the database (see Plugins)

   What the deployment's administrators decide is not in `aspen.toml` but in the database, so every server follows one answer and a change needs no restart: the deployment settings (`app::deployment_settings`, one row of `deployment_settings`, each server keeping a copy that follows changes through the NATS key-value bucket `aspen_settings`). They are the display name and icon (which also name the system account and what authenticators call the deployment), `registration_invite_required`, `require_two_factor`, `bots_enabled` and `bots_max_per_user`, `everyone_mention_limit`, `custom_emoji_limit`, `file_transfers`, `email_required`, `email_verification_required`, `newsletter_enabled`, and the federation gates (`emigration` and `immigration` for users and for bots, each `closed`, `open`, `allowList`, or `blockList`, a shared list, and whether a first arrival from elsewhere needs a registration invite). Holders of Manage deployment settings change them in the dashboard, holders of Manage federation the gates, and `aspen-chat-server settings show|set` any of them; see Administration. A setting belongs in `aspen.toml` when a server needs it to start or it is bound to the deployment's machines and domain, and in the deployment settings when it is a policy administrators choose.

4. Regenerate API schema files:
   ```
   cargo run -- --gen-openapi-schema
   ```
   This writes `openapi.yaml`, `event_schema.json`, and `federation_schema.json` (what deployments exchange; see Federation) to the working directory and then exits. The voice server's `--gen-signal-schema` writes `voice_signal_schema.json` the same way. Both files are gitignored; the client's code generator (`pnpm codegen` in `client/`) reads them from the repository root and can regenerate them itself. Regenerate them whenever API types change.

5. Build for 64-bit ARM (a Raspberry Pi 5, say) on an x86-64 Linux machine:
   ```
   scripts/cross_aarch64.py
   ```
   It fetches a Debian 12 arm64 sysroot (glibc 2.36, so the binaries run on Raspberry Pi OS Bookworm and newer) into `target/cross/`, compiles everything with the clang and lld already installed (the Rust code, the crates' C, and mediasoup's C++ through a generated Meson cross file), and checks the result: AArch64, no glibc symbol newer than the sysroot's, and each binary starting under Debian's user-mode emulator, which it also unpacks there (Meson runs mediasoup's code generator through it during the build). It needs no root. The binaries land in `target/aarch64-unknown-linux-gnu/release/`; on the Pi they need only `libssl3`. `eval "$(scripts/cross_aarch64.py env)"` sets up a shell to run cargo for the target by hand. Every ARM build sets `JEMALLOC_SYS_WITH_LG_PAGE=16`: jemalloc fixes its page size when compiled, a Raspberry Pi 5 boots a 16 KiB-page kernel by default, and a jemalloc built for 4 KiB pages aborts at startup there, while 64 KiB pages serve every smaller kernel page size.

6. Run two deployments that federate, for developing federation:
   ```
   scripts/dev_federation.py up      # then: check, down [--drop]
   ```
   It makes a development certificate authority and certificates for `alpha.localhost` and `beta.localhost` (names under `localhost` resolve to loopback without configuration), and starts the debug builds as `alpha.localhost:8443` and `beta.localhost:8444` over real TLS, each with a database, NATS, and Valkey of its own and gates given with `settings set`, each trusting the authority for calls to the other through `[federation.development]`, and each serving the built web client, or a stand-in when there is none (`scripts/web_client.py`). `check` drives federation between them through the API and the terminal. Everything it makes is under `target/dev-federation/`; trust its `ca.pem` to reach the deployments from a browser.

7. Check that a deployment wakes phones as `spec/push.md` says, against the deployments `dev_federation.py up` runs:
   ```
   scripts/dev_push.py
   ```
   It stands in for a relay and a phone at `push.localhost`, checking each push's signature and decrypting it (Python's `cryptography`).
   `scripts/bench_push.py [--members 10000]` measures the fan-out instead: it seeds a community of that many people with the benchmark seeder, gives each a phone at the same stand-in, has the owner post `@everyone`, reports when the first, half, and last pushes arrived, and purges the run. Run alpha from a release build (`up --alpha-bin target/release`) for figures worth comparing.

8. Check built servers against the services in `docker-compose.yaml`:
   ```
   scripts/smoke_servers.py --bin target/release
   ```
   It migrates a database of its own, runs a NATS of its own, starts both servers on ports of their own (a development stack keeps running beside it), registers and signs in a user, joins a call through the voice server, reads both metrics endpoints, and runs `estimate-capacity`, then drops the database. `scripts/stack.py` is the deployment both this and the next check run against, serving a stand-in web client (`scripts/web_client.py`), with a WebSocket client of the standard library. In a worktree, set `COMPOSE_PROJECT_NAME` to the main checkout's directory name so they find its running services.

   Check that changes to access reach everything already open:
   ```
   scripts/check_permissions.py --bin target/debug
   ```
   It tries each change to who may see or do what (an override, a role given, taken, or deleted, a move, a category deleted, a removal, a ban, a sign-out, a password change, an operator command) and watches every place someone on either side could notice: REST reads, an open event stream, a call on the voice server, attachments. Each feature that grants or shows something adds its scenario here (see the revocation checklist under Standards and Expectations).

9. Watch the API server's async tasks with [tokio-console](https://github.com/tokio-rs/console), which shows each task's busy and idle time, how often it is woken, and which have waited longest, for finding what a slow request or a stalled stream is waiting on:
   ```
   RUSTFLAGS="--cfg tokio_unstable" cargo build --release -p aspen-chat-server --features console --target-dir target/console
   cargo install --locked tokio-console && tokio-console
   ```
   The `console` feature serves the instrumentation on `127.0.0.1:6669` (`TOKIO_CONSOLE_BIND` moves it; keep it on loopback, like the metrics). Tokio compiles that instrumentation only under `--cfg tokio_unstable`, which the feature refuses to build without and which no other build sets; a target directory of its own keeps the flag from rebuilding the usual one. `ASPEN_LOG` filters the log output alone, not what the console reads.

### Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and every pull request: `cargo fmt --check`, `cargo clippy -- -D warnings`, and the workspace tests on x86-64; the example plugins' format, lints, tests, and builds for `wasm32-wasip2` (`plugins/word_filter`, `plugins/forum`, `plugins/calendar`, and `plugins/blackjack`, each a workspace of its own); the client's typecheck, lint, and tests against the schemas that job writes; the Android app's build, its unit tests, and its device tests on an emulator; `scripts/dev_federation.py up --start-services`, `check`, and `scripts/dev_push.py` against the debug build; the Android app's build and JVM tests (`client/packages/mobile/android`; the push handler's end-to-end test needs a device and runs locally); an ARM build in a `debian:bookworm` container on GitHub's arm64 runner (tests included); the cross-compile script on x86-64; and `scripts/smoke_servers.py` on an arm64 runner against both ARM builds, which are kept as artifacts; `scripts/check_permissions.py` runs in the x86-64 job against its debug build, and builds and installs the example plugins. Every job runs the toolchain `rust-toolchain.toml` pins, which lists the `wasm32-wasip2` target the example plugins need, so rustup installs both wherever the repository is built. Clippy warnings fail the build.

`.github/workflows/emoji-font.yml` runs every week: when `googlefonts/noto-emoji` has changed its fonts, it rebuilds the client's bundled emoji fonts with `client/scripts/noto_emoji.py update` and opens a pull request (see `client/docs/architecture/fonts.md`). Its pull requests start CI only when the repository has an `EMOJI_FONT_TOKEN` secret; the workflow file says why.

### CLI Flags

- `--gen-openapi-schema` — Generate schema files and exit; it reads no configuration
- `--no-https` — Disable TLS (development only)
- `-k, --key` / `-c, --cert` — TLS key/cert paths (PEM format)
- `--listen-addr` — Bind address (default `0.0.0.0`, repeatable)
- `--port` — Port (default `443`)
- `--private-worker` — Serve nothing: open no listening socket, need no web client, read no events for connections, and do only the background work every API server shares (mail and digests, voice reports, standing checks, plugins' observers and timers, push; `app::context::Role`). Refuses the listening and TLS flags. Without it a server refuses to start without the web client

Operator subcommands read the same configuration, log to stderr, and exit: `limits suspend|resume|status` and `bench seed|purge`, both described under Benchmarking, `admin grant|revoke|list|allow|deny`, `invites create|list|revoke`, and `settings show|set`, described under Administration, `communities unowned|set-owner`, under Roles and permissions, `voice-servers list|add|set|remove`, under Voice, `federation status|list|add|remove|contact|accept-key|list-add|list-remove|rotate-key --planned|--compromised`, under Federation, and `plugins install|list|show|settings|mode|order|enable|disable|remove|purge`, under Plugins.

## Architecture

Aspen uses PostgreSQL for data persistence, and in order to meet the highly real-time expectations of a chat app Aspen uses NATS JetStream in order to deliver a stream of events. These
events describe changes to the chat environment that the user should be aware of, such as new messages, user online status, and more. NATS JetStream operates in memory, and preserves events
for up to 1 minute. This allows time travel along the event stream up to 1 minute backwards in time. Aspen uses this to alleviate the symptoms of race conditions that may arise inside the
PostgreSQL database when populating the client's initial state. Clients do not connect to NATS JetStream directly, and instead access this information through a WebSocket API provided by the
main API server.

Aspen provides a REST API, along with an autogenerated `openapi.yaml` document which enables client software to automatically generate code which meets the API expectations. Additionally, the JSON types
provided in the WebSocket event stream are described in `event_schema.json` which is a JSON Schema document.

### REST API shape

Everything lives under `/api/v1` (`api::API_PREFIX`). The API is resource-oriented so that browsers, HTTP caches, proxies, OpenAPI tooling, and (eventually) federation all see the shape they expect:

- **Identity lives in the URL, never in the body.** `GET /channels/{channel}`, `PATCH /messages/{message}`, `DELETE /communities/{community}`. Collections are plural nouns; sub-resources hang off their parent (`GET /communities/{community}/channels`, `POST /channels/{channel}/messages`). The literal segment `@me` addresses the calling user (`GET /users/@me`, `PUT /communities/{community}/members/@me`, `PUT /messages/{message}/reactions/{emoji}/@me`).
- **Reads are `GET` with query parameters.** Browsers cannot send a body with `GET`, so no read endpoint may take one. Windowed reads use keyset parameters (`?before=`, `?after=`, `?around=`, `?limit=`) over UUIDv7 ids. A read whose parameters cannot reasonably fit in a query string should be a `POST .../search` today and may move to the HTTP `QUERY` method (RFC 10008) once utoipa, axum, and openapi-typescript all support OpenAPI 3.2.
- **Reads may sideload related records with `?include=`.** A read that supports it accepts a comma-separated list of relationship names (`GET /users/@me/communities?include=channels,categories,members`) and always returns the `{data, included}` envelope from `api::include` (`Sideloaded<T>` for one record, `SideloadedList<T>` for a list), whether or not `include` was given, so the generated client sees one response type per endpoint. `included` is keyed by record type (`communities`, `channels`, `categories`, `users`, `userCommunities`, `messages`, `attachments`, `reactions`, `polls`, `pollVotes`, `ownWriteIns`, `readStates`, `channelMutes`, `categoryCollapses`, `voiceSessions`, `voiceParticipants`, `customEmoji`); a key is present exactly when the caller asked for the relationship that produces it, and holds the same wire records the entity's own endpoints return. The member sample is capped, but the caller's own membership of each community is always among `userCommunities`, because it carries the order of their community list. Each endpoint declares its accepted names as an enum (`api::community::CommunityInclude`, `api::message::MessageInclude`, `api::invite::InviteInclude`, `api::dm::DmInclude`) and parses the parameter with `IncludeSet<E>`, documented in OpenAPI as an array with `style = Form, explode = false`; a name the enum lacks is ignored, never refused, since a newer client or another deployment's may ask for a relationship this version does not have. Sideloads are batched in the `app` layer, one query per relationship however many parent records there are; never load them per record. Reads that do not support `include` return the bare record or array.
- **List parameters use `sort=` and `filter[field]=`.** When a list endpoint gains ordering or filtering, name the parameters `sort` (a field name, `-` prefixed for descending) and `filter[<field>]`, deserialized with `#[serde(rename = "filter[name]")]` on the query struct, so every list reads the same way. The keyset window parameters (`before`, `after`, `around`, `limit`) stay as they are.
- **Writes:** `POST` on a collection creates and returns `201 Created` with a `Location` header (use `api::extract::Created`). `PATCH` partially updates and returns the updated record. `DELETE` returns `204 No Content` (`api::extract::NoContent`). `PUT` is used for idempotent set-membership operations (join a community, add a reaction) and returns `201` when it created something, `200` when it already existed.
- **Updates use JSON Merge Patch semantics.** A field that is absent is unchanged; a field that is present is written, and `null` clears a nullable field. `message_gen` generates `*UpdateRequest` types with `Option<T>` fields, routing nullable fields through `api::extract::double_option` so a JSON `null` arrives as `Some(None)`. The matching `Update` server events skip fields that did not change (`skip_serializing_if`), so clients can apply them as merge patches too. Hand-written update types must follow the same pattern.
- **Errors are RFC 9457 Problem Details** (`application/problem+json`), produced by `api::error::ApiError` and documented as `Problem` on every non-2xx response. `code` (`api::error::ProblemCode`) is the stable discriminator clients branch on; `title` and `detail` are localized prose. `From<app::Error>` handles the common mappings (not found → 404, validation → 400, `Unauthorized` → 403, unique violation → 409, anything else → 500 with a `tracing::error!`). Map to a more specific code in the handler only when the handler knows the context (`usernameTaken`, `inviteCodeTaken`). Add a `ProblemCode` variant rather than inventing ad-hoc error bodies.
- **Extractors:** use `api::extract::{Json, Path, Query}` rather than axum's own so that malformed input yields a Problem instead of plain text. They keep axum's type names on purpose; utoipa recognises handler arguments by name.
- **Auth** is `Authorization: Bearer <session token>` (`api::auth::SessionUser`), declared in OpenAPI as the `bearerAuth` scheme. Every endpoint requires it except registration (`POST /users`), `POST /auth/login` and `/auth/login/second-factor`, `POST /auth/token-refresh`, `GET /auth/methods`, `GET /deployment` (the deployment's display name and icon, which the sign-in screen shows; see Administration), the passkey ceremony endpoints (starting a `signIn` ceremony, reading, completing, and claiming one), the device link endpoints a device that is not signed in uses (starting a `request`, scanning an `offer`, claiming, and cancelling; see Sign-in and security), `GET /registration-invites/{code}` (see Administration), `POST /auth/federated-sign-in` and `GET /federation/icons/{icon}` (see Federation), the three steps of a password reset under `/auth/password-resets` (see Email), and the event stream. Mark authenticated handlers with `security(("bearerAuth" = []))` so generated clients know to send the header, and optionally authenticated ones with `security((), ("bearerAuth" = []))`. A session whose account still owes the server a second factor (see Sign-in and security) is refused by `SessionUser` with `twoFactorEnrollmentRequired`, and one whose email address the deployment requires verified with `emailVerificationRequired` (see Email); the few handlers such a session may reach take `EnrollingSessionUser` instead.
- **Documentation:** every handler carries a `#[utoipa::path]` with a `tag` (the `api::TAG_*` constants), explicit `params` for path and query inputs, and one `responses` entry per status code it can return, each error status with `body = Problem`. The generated client's typed error handling is only as good as these declarations.
- **Rate limits** apply to every endpoint (`app::rate_limit`, `api::rate_limit`). Each endpoint's limits count along dimensions: `global`, `ip`, `user`, `username` (sign-in only), and `per_<param>`, `user_per_<param>`, `ip_per_<param>` for any path parameter. It gets the `default` limits wherever they apply, plus its groups' limits, with its own entry replacing its groups' dimension by dimension; `false` removes a dimension there, the default's included. The built-in limits are `server/src/rate_limits.toml`; `[rate_limits]` in `aspen.toml` is laid over them in code, a limit given there replacing the built-in one whole. Besides the OpenAPI operations, `GET /events` (the event stream), `GET /auth/passkey` (the passkey page, outside `/api/v1`), `GET /.well-known/aspen` (the federation document, also outside it), `GET` and `POST /email/unsubscribe` (the page unsubscribe links open, also outside it), and `GET /invite/{code}` and `GET /{*path}` (the web client's pages, also outside it) can be named. The rules are compiled against the real routes at startup, so an unknown endpoint, a parameter the path lacks, or a user limit on an anonymous endpoint stops the server with a config error. Buckets are GCRA timestamps in Valkey, shared by every API server, and a Valkey outage lets requests through with a logged error, except on the endpoints `fail_closed` lists (those that guess a password, a code, or an invite) and for the username limit, which it refuses with `503` `serverBusy`. Address, global, and parameter limits are checked by a route layer before the handler; user limits in the session extractor, once the token names the user; the username limit in the sign-in handler; the web client's pages check their own in their handler, and over them serve the page without reading what it previews as rather than refusing it. A refusal is `429` `rateLimited` with `Retry-After`, and `api::rate_limit::document_rate_limits` adds that response to every operation in the OpenAPI document, so handlers do not declare it. The client address is the TCP peer, or, when the peer is listed in `trusted_proxies`, the right-most `X-Forwarded-For` entry that is not itself a trusted proxy (read with or without a port; an entry that is not an address ends the reading, and the client is the trusted proxy that passed it on); IPv6 clients count by their `/ipv6_prefix` (64) network. When adding an endpoint that writes, fans out, fetches from elsewhere, or can be used to guess something, give it its own limits in `rate_limits.toml`.
- **Wire naming** is camelCase everywhere (`#[serde(rename_all = "camelCase")]` on every request, response, and record type).
- **Other versions and forks call this API too** (clients of other deployments, since federation), so the API changes only by addition within `/api/v1`, and a request another deployment's client may send is never refused for a field this server does not know: `deny_unknown_fields` is only for requests that just this deployment's own clients send (the Administration Dashboard, bot management, security settings). `spec/federation.md` holds the rules.

The WebSocket event stream is served at `/api/v1/events` (`server/src/api/event_stream.rs`). It is not part of the OpenAPI document; `event_schema.json` describes every frame in both directions (root type `EventStreamProtocol`, with `ClientMessage` and `ServerMessage`). Browsers cannot set headers on a WebSocket upgrade, so authentication is in-band: the client's first frame must be `{"type":"identify","sessionToken":"…","resumeAfter":<sequence>?}` within ten seconds, or the server closes with code 4408 (4400 for a malformed frame, 4401 for a bad token), after sending an `error` frame with a localized `detail`. An open stream closes with 4401 when its sign-in ends (after the `signInsEnded` event) or expires, 4410 when its account is banned (after `accountBanned`), 4403 when the deployment starts requiring a second factor it lacks, and 4428 when the deployment requires a verified email address its account does not have; one more stream than `[limits] max_event_streams_per_user` or `max_event_streams_per_address` allow on that server is closed with 4429. After it the client sends only `{"type":"activity"}` frames (see Presence under Event routing). The server replies `{"type":"ready","userId":…,"resumed":bool}` and then `{"type":"event","sequence":N,"event":{…}}` frames. `sequence` is the JetStream stream sequence; a client that reconnects with `resumeAfter` set to the last sequence it processed gets exactly the events it missed and `resumed: true` when that position is still within `MAX_EVENT_AGE`, otherwise the usual replay window and `resumed: false`, which tells it to rebuild its cached state from REST. The server pings every 30 seconds and drops peers that miss two pongs. Each `event` payload is an object tagged with `serverEvent` (the entity) and `type` (`create`, `update`, `delete`) with the record's fields at the top level, plus a few custom events such as `voiceSpeaking`. The frame carries `eventId` when the event may reach the client more than once.

### Layers

The server is split into three layers:

- **`server/src/api/`** — HTTP handlers and request/response types. Strictly concerned with HTTP interactions. Should not contain business logic, database queries, or message broker interactions. Think of this as a frontend to the app layer. Types the app layer needs (such as `app::channel::MessageWindow`) belong in `app`, not here; `api` depends on `app`, never the reverse, except for the `message_gen`-generated records and request types in `api::message_enum` which both layers share.
- **`server/src/app/`** — Business logic. Responsible for checking permissions and carrying out the operations expected of each API endpoint. Contains ID types, the `MaybeLoaded` lazy-loading pattern, event publishing, and shared context (`GlobalServerContext`).
- **`server/src/database/`** — Auto-generated Diesel schema code. Files here should not be edited by hand; alter them by adding a migration in `migrate/src/migrations/`, applying it with `cargo run -p aspen-migrate -- up`, and then running `diesel print-schema > src/database/schema.rs` from `server/`.

### Request Flow

Client REST request → API handler (validate, parse) → App function (permissions, business logic, database) → Publish event to NATS → WebSocket stream delivers event to connected clients.

### Architecture reference

How each feature works is written up in `docs/architecture/`, one file per feature. Before working on a feature, read its file, and keep it describing the code as it stands in the same commit, as every comment must. Where this file or one of those says "see Bots" or "described under Voice", it means the file of that title.

- `docs/architecture/background-tasks.md` — the poll closer, the voice report listener and reaper, closing polls, message kinds, and poll write-ins
- `docs/architecture/reactions.md` — reactions, their canonical emoji, summaries, and reactor lists
- `docs/architecture/threads-and-dms.md` — threads, echoes, DMs and group DMs, their calls and rings, and the system account
- `docs/architecture/sign-in-and-security.md` — password sign-in, second factors, recovery codes, reverification, password hashing limits, passkeys, and signing in from another device by a QR code
- `docs/architecture/user-preferences.md` — account-scoped preferences stored as one JSON object
- `docs/architecture/read-positions.md` — read positions, unread, and unread tag counts
- `docs/architecture/muting.md` — muting channels and DMs
- `docs/architecture/collapsed-categories.md` — folded categories that follow a user between devices
- `docs/architecture/search.md` — message search and its indexes
- `docs/architecture/notifications.md` — notification levels and what the apps tell of
- `docs/architecture/tagging.md` — tagging members, roles, and everyone, and what counts
- `docs/architecture/bots.md` — bots, their tokens, links, roles, and commands
- `docs/architecture/custom-emoji.md` — a community's own emoji in text and reactions
- `docs/architecture/blocking.md` — what a block does on the server
- `docs/architecture/administration.md` — deployment roles, deployment moderation and its log, bans from the deployment, registration invites and dual invites, the deployment settings, and fleet health
- `docs/architecture/voice.md` — voice servers, sessions, reports, the reaper, and the voice server's media, signalling, and limits
- `docs/architecture/file-transfers.md` — file transfers in calls, the STUN and TURN relay, and their record
- `docs/architecture/federation.md` — every phase of federation: identity and keys, gates and lists, the directory, signing in abroad, protocol versions, DMs across deployments, and standing
- `docs/architecture/push.md` — waking phones through Web Push
- `docs/architecture/attachment-previews.md` — previews of pictures and videos' posters, how they are made and queued, and messages held for them
- `docs/architecture/email.md` — email addresses and their verification, the shown address, password reset by email, the outbox, the daily digest, the newsletter, and unsubscribing
- `docs/architecture/event-routing.md` — event subjects, the event feed and its shards, visibility, `publish_event` scopes, and presence
- `docs/architecture/benchmarking.md` — `aspen-bench`, seeding and purging runs, suspending rate limits, and the metrics both servers export
- `docs/architecture/reports.md` — reports of messages, profiles, and nicknames, their categories, cases and their review, warnings, and what deleting a message keeps
- `docs/architecture/message-links.md` — links between messages, what each reader finds at them, and how they are sideloaded
- `docs/architecture/roles-and-permissions.md` — community permissions, roles, overrides, ranking, bans, the everyone mention limit, and member search
- `docs/architecture/web-client.md` — the web client every API server serves at `public_url`, its files and caching, and its pages' Open Graph tags: the deployment's, and an invite's community's
- `docs/architecture/plugins.md` — installing plugins, where they run, the sandbox, intercepting and observing, what the host answers and as whom, annotations, storage, routes, plugin events, principals, channel types and views, timers, notices, cards, and capability URLs

### Event Ordering Guarantee

Updates must be sent to NATS JetStream in the same order they were committed to PostgreSQL. The ordering can be arbitrary, but it must be consistent between the two. To achieve this, `UPDATE` SQL queries should occur inside a SQL transaction. The correct sequence is:

1. Begin a SQL transaction.
2. Execute the update query.
3. Before committing the transaction, publish the event to NATS JetStream.
4. Wait for JetStream to acknowledge the event.
5. Only then commit the SQL transaction.

This ensures that if JetStream rejects or fails to acknowledge the event, the database change is rolled back, and the two sources of truth never diverge in ordering.

## `message_gen` Procedural Macro

`message_gen` is a procedural macro system which generates, for each record type Aspen handles, the wire record, the REST request bodies, and the server events, all from one definition so they cannot drift apart. Request bodies are used with the REST API, while server events are used with the WebSocket event API.

`message_gen` is invoked inside of `server/src/api/message_enum.rs`. The macro is applied to an enum where each variant defines an entity and its fields. From this single definition, the macro generates:

- **Record structs** — plain data structs (with Serialize, ToSchema, JsonSchema) in `api::message_enum`, returned by REST reads and carried by `Create` events
- **Request structs** in `api::message_enum::request` — `{Entity}CreateRequest` (every client-settable field) and `{Entity}UpdateRequest` (updatable fields as `Option<T>`, merge-patch semantics). Identifiers and parent references are never in a body; they come from the URL path or the session. Neither struct is generated when it would be empty (a `React` is created by `PUT` on a URL that already names everything).
- **ServerEvent enum** in `api::message_enum::server_event` — with `Create`, `Update`, `Delete` variants per entity, used in the WebSocket event stream

### Field Annotations

- `#[message_gen(id)]` — Marks the primary identifier field. Required for each entity. Server-authoritative by default (server generates the ID).
- `#[message_gen(id = "client_authoritative")]` — The client provides the ID value (through the URL path, for example an emoji or an invite code).
- `#[message_gen(parent)]` — The owning record, supplied through the URL path on create (`POST /channels/{channel}/messages` sets `Message.channel_id`). Implicitly permanent; excluded from both request bodies.
- `#[message_gen(permanent)]` — Field is set at creation and cannot be updated. Included in `CreateRequest` but not `UpdateRequest`.
- `#[message_gen(server_authoritative)]` — Server controls this field entirely. Clients cannot set or modify it (e.g., timestamps, author IDs).
- `#[message_gen(server_authoritative = "mutable")]` — As above, but the server may change the field after creation (e.g., `Message.edited_at`), so the `Update` server event carries it as an `Option<T>` alongside the client-updatable fields. Set it in the event only when it changed.
- `#[message_gen(default)]` — The field may be absent from a create request and from the record (in the schemas), and is then its type's default (`Role.hoist`). A non-nullable field added to an entity that already exists needs it, since the API changes only by addition: clients that predate it create without it, and newer clients read records from deployments that predate it.
- `#[message_gen(secret)]` — Accepted on create but excluded from records and events. Used for genuinely secret inputs (passwords, invite codes) and for inputs the server transforms rather than stores (`Poll.duration_seconds`, which becomes `closes_at`).

### Entity-Level Annotations

- `#[message_gen(no_commands)]` — Skip generating REST CRUD commands for this entity (e.g., `UserStatus` which is managed internally).
- `#[message_gen(no_events)]` — Skip generating WebSocket server events for this entity (e.g., `Attachment`, `Icon`).

## Adding a New Entity

1. **Create a migration**:
   - Scaffold with `cargo run -p aspen-migrate -- new <slug>`. This creates a `migrate/src/migrations/m<ts>_<slug>/{mod.rs,up.sql,down.sql}` directory AND registers the module (`pub mod m<ts>_<slug>;` in `migrations/mod.rs`, `&migrations::m<ts>_<slug>::M,` appended to `MIGRATIONS` in `registry.rs`).
   - Edit `up.sql` and `down.sql`. For migrations that need real Rust work (data backfills, calls into `MediaStore`, etc.), replace `mod.rs` with a hand-written `impl Migration` instead of `SqlMigration`.
   - Apply it: `cargo run -p aspen-migrate -- up`.
   - Regenerate `server/src/database/schema.rs`: `cd server && diesel print-schema > src/database/schema.rs`.
2. **Add the entity to `message_enum.rs`** — add a new variant to the `MessageEnumSource` enum with appropriate field annotations.
3. **Create `server/src/api/<entity>.rs`** — implement the HTTP handler functions following the REST API shape above (`POST /<entities>` or `POST /<parents>/{parent}/<entities>`, `GET`/`PATCH`/`DELETE /<entities>/{entity}`, plus any list endpoints), each with a complete `#[utoipa::path]`. Register the module in `server/src/api/mod.rs` and add a `TAG_*` constant.
4. **Create `server/src/app/<entity>.rs`** — implement the business logic functions. Register the module in `server/src/app/mod.rs`.
5. **Wire routes in `server/src/api/mod.rs`** — add `.routes(routes!(...))` calls in the `start` function.
6. **Add an ID type** (if needed) in `server/src/app/mod.rs` using the `id!` macro.
7. **Decide who may use it** — check permissions in the app function (`app::permissions::require_member` or `channel_access`, then `require`), and document `403` on the handler when a permission can refuse. Answer the revocation checklist (under Standards and Expectations) for it, in its architecture file, and add its scenario to `scripts/check_permissions.py`.
8. **Consider its rate limits** — the defaults cover every endpoint, but one that writes or is expensive usually wants its own entry in `server/src/rate_limits.toml`.
9. **Regenerate schemas** — run `cargo run -- --gen-openapi-schema` and commit the updated `openapi.yaml` and `event_schema.json`.

## Error Handling

Aspen uses both `thiserror` and `anyhow`, each with a distinct purpose:

- **`thiserror`** (`app::Error` enum in `server/src/app/error.rs`) — for errors that calling code might conceivably want to catch and handle. Each variant represents a specific failure mode (database error, connection pool error, config error, etc.) and can be matched on.
- **`anyhow`** — for errors that are fatal and require intervention from a server administrator or developer. These represent situations where the server cannot reasonably recover on its own.

The project defines `app::Result<T>` as an alias for `std::result::Result<T, app::Error>`. Error types use `#[from]` attributes for ergonomic `?` conversion.

## Testing

Testing infrastructure is still in its early stages. The long-term goal is rigorous test suites. Current guidance:

- Unit tests go inline as `#[cfg(test)] mod tests` within the module they test.
- Integration tests will go in a top-level `tests/` directory once the test infrastructure matures.
- Tests should hit a real PostgreSQL database via `docker-compose`, not mocks, to ensure database behavior matches production.

## Standards and Expectations

You are an expert level architect and senior software engineer. You will think through your actions before you take them, and build out a maintainable and robust system which will survive
scrutiny from the most talented software professionals in the world. Consider the performance and long term implications of your solutions before you implement them.
If you receive a request which is ambiguous, you will seek clarification. If you receive a request which is ill advised, you will recommend against it. Time is precious, and you'd rather
do it right the first time. You will always check your work by running `cargo clippy` and fixing any new problems.

### When access is given or taken away

Most security holes in a chat server are stale state: someone loses a permission, or a session, and something already open keeps acting on the old answer. Every feature that shows something to someone, or lets them do something, answers these in its architecture file in the commit that adds it, and `scripts/check_permissions.py` gains a scenario checking the answers:

1. **Who can observe it, and by which routes?** REST reads (alone and sideloaded), the event stream (which subject, which `Aspen-Channel` or `Aspen-Requires`), push, the voice server, search, message links, the client's cache.
2. **What decides it, and where is that checked?** A read of one channel's contents goes through `channel_access`, whose `ChannelAccess` only that function can make, so taking one as an argument proves the check. A read listing what lies in several channels takes a `Visibility` and keeps only what it allows (`read_communities_channels`, `read_community_mutes`); never filter visibility in the `api` layer, where it is forgotten. An event decides its readers by its scope.
3. **When the deciding permission is lost, what happens to what is already open?** Think through each way it can go: a role's permissions edited, a role taken or deleted, an override, a channel moved, a category deleted, a member removed, banned, or leaving, the owner changed, a deployment role, a sign-out, a password change, a ban from the deployment, an account deleted. For each, say what becomes of open event streams (the side losing access receives the change and nothing after it), calls (`app::events::rechecks_of`), phones (`app::push`), server caches, and the client's cache (`RecordStore`'s pruning).
4. **When it is gained, how does a client already open find out without a reload?** Events that change who may view a channel reach both sides (`FeedEvent::before`); the client looks up what it hears of and lacks (`AspenSync`).
5. **Does every path that changes it announce it?** That includes operator commands (`app::events::Publisher`), background tasks (inside `app::events::noting`), and database cascades: either the cascade's effect is announced, or every reader (the event feed's model, the client's store) infers it from the event that caused it.
6. **Is it published inside the transaction that makes the change?** Then a rollback is answered by `communityResync` for the communities it was published to and `userResync` for the users (`app::events::settle`).

The code holds some of these on its own: `expected_kind` and `rechecks_of` match every event with no wildcard, so a new event cannot be published until its routing and its effect on calls are decided; `rechecks_of` runs those rechecks after the work commits, so no call site needs to remember them; `ChannelAccess` and `Visibility` cannot be made without the check they stand for. Prefer extending these to adding a rule here.

### Implement the trait rather than working around it

When code would be simpler with a trait a type does not have, give the type the trait: derive it, implement it, or teach `message_gen` to derive it on what it generates. Do not clone field by field because a type is not `Clone`, sort by an inner field because an id is not `Ord`, round-trip through `serde_json` to parse a name because an enum is not `FromStr`, or keep a second table of names beside the serde attributes. For enums whose wire names are also their names in headers, the terminal, and errors, `app::wire_name_traits!` gives `Display` and `FromStr` through the serde names, so those stay the only list. The ID types (`id_type!`) are `Ord`, `message_gen`'s records and server events are `Clone`, and its update requests are `Default`, a patch that changes nothing.

Crates that provide these traits are welcome, and the workspace uses several:
- `bitflags` for sets of flags. `Permissions` and `DeploymentPermissions` are stored as `BIGINT` through `app::bigint_sql_traits!`.
- `strum` for names and lists of variants that serde does not give (`IntoStaticStr`, `VariantNames`, `VariantArray`).
- `smart-default` for config sections whose defaults are written on their fields, with `#[serde(default)]` so a missing key takes them.
- `humantime` for durations typed at the terminal.

A conversion that takes one value and nothing else is `impl From`, not a `*_to_api` function.

## Comments document the current code, not its history

Every comment and docstring anywhere in this repository — Rust in `server/`, TypeScript in `client/`, SQL in `migrations/`, config in `docker-compose.yaml`, and any future language we add — must describe the code as it stands in the tree right now. Do not write comments that contrast the current implementation with an earlier one, explain why today's code is "better than" or "replaces" something that used to exist, or cite removed helpers / classes / functions / modules by name as parallels or fallbacks. A reader opening the file a year from now has no way to resolve references like "the previous threaded implementation", "the client used to scrape this itself", "the old command-response enums", or "`_coerce_overrides` used to need"; those references become dead weight the moment the commit that removed the original code lands, and they actively mislead anyone grepping for the named symbol.

Concretely, while editing:

- State invariants, rationales, and trade-offs as present-tense facts about the current code. "`None` means no change" is good; "`None` means no change, matching the previous hand-coded merge semantics" is not.
- Cross-layer references must point at *live* code. "`LinkPreviewImageCache` mirrors `IconCache`" is fine because both classes exist today; "`LinkPreviewImageCache` replaces the old `LinkPreviewCache`" is not.
- If a non-obvious choice is only defensible by comparing to an alternative, compare to the *alternative* (what the code could have done instead and why it doesn't), not to a previous revision of this file.
- When you refactor or delete code, sweep the comments in the same commit. A comment that names a helper is invalidated the moment that helper is renamed or removed; do not leave it behind to be cleaned up later.
- This rule applies equally to this `AGENTS.md` and to `client/AGENTS.md`. If a rule here is justified by a historical bug, describe the bug and the invariant it implies — do not describe the removed fix.
- The one narrow exception is historical context that a reader genuinely needs in order to understand why a rule is load-bearing (for example, "we had this exact freeze once, don't reintroduce it"). Even then, describe the *bug*, not the removed code that caused it.

If you catch a stale "previously / used to / legacy / the old X / the client used to scrape this" comment while you're editing nearby code, fix it. Do not wait for a dedicated cleanup pass — those do not happen.

## Localization

Aspen uses [`rust-i18n`](https://crates.io/crates/rust-i18n) for internationalization of all client-facing strings. The `i18n!` macro is invoked in `server/src/main.rs`, and translation files live in `server/locales/`.

Each request is answered in its own language (`app::locale`): a layer around every route negotiates `Accept-Language` against the locales the server has (a language matched whole, then with its subtags taken off, `en-GB` finding `en`; English when none matches), holds the result in a task-local for as long as the request is handled, and names it in `Content-Language` (with `Vary: Accept-Language`). The event stream takes `?locale=` on its upgrade URL instead, since a browser cannot set that header on a WebSocket. Work outside a request (background tasks) speaks English. The client sets both from its own language setting, so a deployment's errors read in the language the app shows. Two pseudo-locales are made from English at run time (`app::locale::PseudoLocales`, laid over the catalogue as a rust-i18n backend): `en-XA` accents every letter, doubles every vowel, and brackets the whole, which shows untranslated text and whether a layout takes longer strings, and `ar-XB` sets each word right to left, which exercises right-to-left layout. The client has the same two (`client/packages/app/src/i18n/pseudo.ts`), and both pass `spec/pseudo_locale_vectors.json`; change the three together.

### Rules

- **Never hardcode English text** in API responses, validation errors, or any string returned to clients. Use the `t!()` macro instead: the crate's own `crate::t!`, which is `rust_i18n::t!` in the request's locale, never `rust_i18n::t!` itself, which would answer every request in one language.
- Log messages (`tracing::error!`, `tracing::warn!`, etc.) are **not** localized — they are developer-facing and should remain in English.
- **Every error a person reads says what went wrong and what to do about it**, in words the person who sees it can act on: "%{domain}'s certificate has expired. Its administrators need to renew it", never "could not be reached" when the cause is known. Name the thing (the domain, the limit, the permission) and the one who can fix it (the reader, the other side's administrators, this server's). Classify failures where they happen so the specific cause reaches the message (`app::federation::fetch::failure` does this for requests to other deployments), and give a refusal its reason in `detail` rather than a generic title. A Problem's `title` may stay general because its `detail` carries the specifics.
- Translation keys use **camelCase** (e.g., `inviteCodeLength`, `tryAgainLater`).
- The `t!()` macro returns `Cow<'static, str>`. The `app::Error::Validation` variant and all manually-defined API response error/reason fields accept `Cow<'static, str>` to match. Do not call `.to_string()` on `t!()` output.

### Adding a new client-facing string

1. Add the key and English text to `server/locales/en.yml`.
2. Use `t!("keyName")` in your Rust code, adding `use crate::t;` to the file's imports.
3. If the string contains interpolated values, use `t!("keyName", field = value)` and reference them in the YAML as `"... %{field} ..."`.

### File layout

- `server/locales/en.yml` — English translations (the default, and the only catalogue besides the pseudo-locales made from it). Additional locale files (e.g., `fr.yml`, `de.yml`) go alongside it and are negotiated with no further change; the client needs a catalogue of its own for each (`LANGUAGES` in `client/packages/app/src/i18n/locales.ts`).

## Git Policy

You are not to commit anything to `main` or push any branch to `origin`. If your work is deemed to be of sufficient quality, a human will send it on for you.