Aspen is a free and open source federated community chat solution implemented to the highest code quality standards. Aspen takes a lot of inspiration from the Discord chat platform, but aims
to reach an even higher degree of quality and user delight. Aspen clients will be made for iOS, Android, Windows, Mac, and Linux.

Aspen seeks to support community text chat, voice calls, and video feeds from cameras, screen sharing, and videogame capture via libobs.

Aspen rigidly adheres to DRY, and is split into three layers in implementation. This repo contains the server-side implementation in `server/` and the cross-platform client in `client/` (see `client/AGENTS.md`).
Once complete, Aspen will have rigorous test suites, hardware benchmarking programs, and infrastructure as code to aid with deploying its many docker containers.

Aspen aims to be horizontally scalable, providing a user experience suitable for millions of users, and also to be just as suitable for communities containing less than a dozen users.

Federation is a long-term goal. The `other_server_auth_token` table exists as a placeholder, but federation design is not yet finalized. Do not build federation features without explicit direction.

## Tech Stack

- **Web framework:** Axum (with `utoipa-axum` for OpenAPI integration)
- **Async runtime:** Tokio (multi-threaded)
- **ORM:** Diesel with `diesel-async` and `deadpool` connection pooling
- **Database:** PostgreSQL
- **Message broker:** NATS JetStream (via `async-nats`)
- **Password hashing:** Argon2
- **ID generation:** UUID v7
- **Serialization:** Serde + `serde_json`
- **OpenAPI generation:** utoipa
- **JSON Schema generation:** schemars
- **TLS:** rustls (with self-signed cert generation via `rcgen` for development)

## Building and Running

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

3. Run the server:
   ```
   cargo run
   ```
   The server reads configuration from `aspen.toml` (or environment variables with the `ASPEN_` prefix). Key config values:
   - `database_url` — PostgreSQL connection string
   - `nats_url` — NATS server address
   - `nats_auth_token` — NATS authentication token
   - `[voice]` — `token_secret` shared with the voice servers, the failure threshold and window, the join token lifetime, the candidate cap, the two silence limits, the idle call limit, and `[[voice.servers]]` entries (`name`, `url`, `capacity`) seeded at startup
   - `[cors] allowed_origins` — page origins allowed to call the API from a browser (the web client, Electron, and Capacitor shells are all browsers). `["*"]` allows every origin and is acceptable only in development; an empty list (the default) sends no CORS headers, which is correct when the API and the web client share an origin.

4. Regenerate API schema files:
   ```
   cargo run -- --gen-openapi-schema
   ```
   This writes `openapi.yaml` and `event_schema.json` to the working directory and then exits. The voice server's `--gen-signal-schema` writes `voice_signal_schema.json` the same way. Both files are gitignored; the client's code generator (`pnpm codegen` in `client/`) reads them from the repository root and can regenerate them itself. Regenerate them whenever API types change.

### CLI Flags

- `--gen-openapi-schema` — Generate schema files and exit
- `--no-https` — Disable TLS (development only)
- `-k, --key` / `-c, --cert` — TLS key/cert paths (PEM format)
- `--listen-addr` — Bind address (default `0.0.0.0`, repeatable)
- `--port` — Port (default `443`)

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
- **Reads may sideload related records with `?include=`.** A read that supports it accepts a comma-separated list of relationship names (`GET /users/@me/communities?include=channels,categories,members`) and always returns the `{data, included}` envelope from `api::include` (`Sideloaded<T>` for one record, `SideloadedList<T>` for a list), whether or not `include` was given, so the generated client sees one response type per endpoint. `included` is keyed by record type (`communities`, `channels`, `categories`, `users`, `userCommunities`, `attachments`, `polls`, `pollVotes`, `voiceSessions`, `voiceParticipants`); a key is present exactly when the caller asked for the relationship that produces it, and holds the same wire records the entity's own endpoints return. The member sample is capped, but the caller's own membership of each community is always among `userCommunities`, because it carries the order of their community list. Each endpoint declares its accepted names as an enum (`api::community::CommunityInclude`, `api::message::MessageInclude`, `api::invite::InviteInclude`) and parses the parameter with `IncludeSet<E>`, documented in OpenAPI as an array with `style = Form, explode = false`. Sideloads are batched in the `app` layer, one query per relationship however many parent records there are; never load them per record. Reads that do not support `include` return the bare record or array.
- **List parameters use `sort=` and `filter[field]=`.** When a list endpoint gains ordering or filtering, name the parameters `sort` (a field name, `-` prefixed for descending) and `filter[<field>]`, deserialized with `#[serde(rename = "filter[name]")]` on the query struct, so every list reads the same way. The keyset window parameters (`before`, `after`, `around`, `limit`) stay as they are.
- **Writes:** `POST` on a collection creates and returns `201 Created` with a `Location` header (use `api::extract::Created`). `PATCH` partially updates and returns the updated record. `DELETE` returns `204 No Content` (`api::extract::NoContent`). `PUT` is used for idempotent set-membership operations (join a community, add a reaction) and returns `201` when it created something, `200` when it already existed.
- **Updates use JSON Merge Patch semantics.** A field that is absent is unchanged; a field that is present is written, and `null` clears a nullable field. `message_gen` generates `*UpdateRequest` types with `Option<T>` fields, routing nullable fields through `api::double_option` so a JSON `null` arrives as `Some(None)`. The matching `Update` server events skip fields that did not change (`skip_serializing_if`), so clients can apply them as merge patches too. Hand-written update types must follow the same pattern.
- **Errors are RFC 9457 Problem Details** (`application/problem+json`), produced by `api::error::ApiError` and documented as `Problem` on every non-2xx response. `code` (`api::error::ProblemCode`) is the stable discriminator clients branch on; `title` and `detail` are localized prose. `From<app::Error>` handles the common mappings (not found → 404, validation → 400, `Unauthorized` → 403, unique violation → 409, anything else → 500 with a `tracing::error!`). Map to a more specific code in the handler only when the handler knows the context (`usernameTaken`, `inviteCodeTaken`). Add a `ProblemCode` variant rather than inventing ad-hoc error bodies.
- **Extractors:** use `api::extract::{Json, Path, Query}` rather than axum's own so that malformed input yields a Problem instead of plain text. They keep axum's type names on purpose; utoipa recognises handler arguments by name.
- **Auth** is `Authorization: Bearer <session token>` (`api::auth::SessionUser`), declared in OpenAPI as the `bearerAuth` scheme. Every endpoint requires it except registration (`POST /users`), `POST /auth/login`, `POST /auth/token-refresh`, and the event stream. Mark authenticated handlers with `security(("bearerAuth" = []))` so generated clients know to send the header.
- **Documentation:** every handler carries a `#[utoipa::path]` with a `tag` (the `api::TAG_*` constants), explicit `params` for path and query inputs, and one `responses` entry per status code it can return, each error status with `body = Problem`. The generated client's typed error handling is only as good as these declarations.
- **Wire naming** is camelCase everywhere (`#[serde(rename_all = "camelCase")]` on every request, response, and record type).

The WebSocket event stream is served at `/api/v1/events` (`server/src/api/event_stream.rs`). It is not part of the OpenAPI document; `event_schema.json` describes every frame in both directions (root type `EventStreamProtocol`, with `ClientMessage` and `ServerMessage`). Browsers cannot set headers on a WebSocket upgrade, so authentication is in-band: the client's first frame must be `{"type":"identify","sessionToken":"…","resumeAfter":<sequence>?}` within ten seconds, or the server closes with code 4408 (4400 for a malformed frame, 4401 for a bad token), after sending an `error` frame with a localized `detail`. The server replies `{"type":"ready","userId":…,"resumed":bool}` and then `{"type":"event","sequence":N,"event":{…}}` frames. `sequence` is the JetStream stream sequence; a client that reconnects with `resumeAfter` set to the last sequence it processed gets exactly the events it missed and `resumed: true` when that position is still within `MAX_EVENT_AGE`, otherwise the usual replay window and `resumed: false`, which tells it to rebuild its cached state from REST. The server pings every 30 seconds and drops peers that miss two pongs. Each `event` payload is an object tagged with `serverEvent` (the entity) and `type` (`create`, `update`, `delete`) with the record's fields at the top level, plus a few custom events such as `userStatus`.

### Layers

The server is split into three layers:

- **`server/src/api/`** — HTTP handlers and request/response types. Strictly concerned with HTTP interactions. Should not contain business logic, database queries, or message broker interactions. Think of this as a frontend to the app layer. Types the app layer needs (such as `app::channel::MessageWindow`) belong in `app`, not here; `api` depends on `app`, never the reverse, except for the `message_gen`-generated records and request types in `api::message_enum` which both layers share.
- **`server/src/app/`** — Business logic. Responsible for checking permissions and carrying out the operations expected of each API endpoint. Contains ID types, the `MaybeLoaded` lazy-loading pattern, event publishing, and shared context (`GlobalServerContext`).
- **`server/src/database/`** — Auto-generated Diesel schema code. Files here should not be edited by hand; alter them by adding a migration in `migrate/src/migrations/`, applying it with `cargo run -p aspen-migrate -- up`, and then running `diesel print-schema > src/database/schema.rs` from `server/`.

### Request Flow

Client REST request → API handler (validate, parse) → App function (permissions, business logic, database) → Publish event to NATS → WebSocket stream delivers event to connected clients.

### Background tasks

Four tasks run alongside the request handlers, all started from `make_router` / `GlobalServerContext::new`: the presence expiry listener (`app::user_status::spawn_expiry_listener`), which turns Valkey key expiries into `userStatus` events; the poll closer (`app::poll::spawn_closer`), which every few seconds closes polls whose `closes_at` has passed, publishes their final tally, and posts the `poll_closed` message; the voice report listener (`app::voice::spawn_report_listener`); and the voice reaper (`app::voice::spawn_reaper`), both described under Voice below. The closer selects due polls `FOR UPDATE SKIP LOCKED`, so several server instances can run it without closing a poll twice, and vote changes lock the poll row too, so the tallies in successive events never go backwards. Messages have a `kind` (`standard`, `poll`, `poll_closed`); the poll kinds carry no content and name their poll in `poll`, and the client renders them from the poll record.

### Voice

Calls run on voice servers, separate processes registered in the `voice_server` table (seeded from `[[voice.servers]]` in `aspen.toml` by name at startup, and managed through `/voice-servers`). The crate `voice_protocol/` is what the API server and the voice servers share: the join token (HMAC-SHA256 under `[voice] token_secret`) and the control messages. A client never authenticates with a voice server directly: `POST /channels/{channel}/voice/join` returns a token naming the user, the channel, and the candidate servers, plus those candidates. A channel already in a call is offered only that call's server; otherwise every enabled server with room that has reported within `offer_silence_seconds` (a server that has never reported has not started) is a candidate, at most `candidate_limit` (ten) of them chosen at random for now. The client measures its own latency to each candidate and tries them nearest first; a server that fails to start the session is reported with `POST /voice-servers/{server}/failures`, which counts distinct users within `failure_window_seconds` and disables the server at `failure_threshold` (five) until an operator enables it again.

Voice servers publish `VoiceReport`s on the NATS subject `aspen.voice.report`; the API servers subscribe in a queue group so each report is handled once, and `app::voice::apply_report` turns it into rows and client events inside one transaction: `voiceSession` create and delete, `voiceParticipant` create, update, and delete, and the custom `voiceSpeaking` event. A session exists from the first participant's report until the last one leaves. Voice servers also report their load every fifteen seconds. A server silent for `offer_silence_seconds` (a minute) is not offered to new joiners, a call bound to such a server is skipped when someone joins its channel (they get fresh candidates), and the reaper ends every session on a server silent for `session_silence_seconds` (a day), so a dead voice server never leaves phantom calls while a brief outage of the report link does not end anyone's call. The reaper also ends any call that has gone `idle_session_seconds` (a day) without ever holding two people at once, so a forgotten client cannot hold a voice server slot. A voice server reports a session only once it holds the room, so the session it reports is the channel's call from then on: any session already recorded for that channel, whether the same server's room lost to a restart or another server's that the joiner could not reach, is ended with reason `serverLost`, which sends its participants to rejoin, and their offers now name the reporting server. Every ending publishes a `voiceSessionEnded` event with a reason (`empty`, `serverLost`, `idle`, `serverRemoved`) just before the session's `delete`; a client that was in an idle-ended call shows its user a dialog saying the call was ended to free server resources. Nested `[voice]` keys can be set from the environment with a double underscore, `ASPEN_VOICE__IDLE_SESSION_SECONDS=30` for instance, which is how tests shorten the limits. Community reads sideload calls with `include=voice`. Commands from the API server to a voice server (mute, kick) travel on `aspen.voice.command.{server}`. `PATCH /channels/{channel}/voice/participants/{user}` with `{"muted": bool}` server-mutes someone and `DELETE` on the same path removes them from the call; both answer `202 Accepted`, because the voice server applies the command and its report is what changes the participant record, so the caller watches for the `voiceParticipant` event rather than reading the response as the result. Under the Two Insanities anyone may do either.

#### The voice server

`voice_server/` is the media process: a mediasoup SFU (each participant uploads once and the server forwards to everyone else) with an HTTP server for the health check clients measure latency against (`GET /health`, CORS open) and the signalling socket (`GET /ws`). It reads `voice_server.toml` from the working directory (gitignored; `id` is the server's row in the registry, `token_secret` is the API server's, plus NATS, `listen_addr`, `workers`, and `[rtc]` with the media interface `ip`, `announced_address`, and the port range), with `ASPEN_VOICE_SERVER_` environment overrides using `__` for nesting. The announced address is what goes into ICE candidates: set it for a server behind NAT; left unset with `ip` bound to every interface (the default), the host's primary interface address is announced, which suits a development machine. It must never be a loopback address, and the server refuses to start with one: Firefox does not pair its own host candidates with a loopback peer, so signalling would succeed and media would never flow. Run it with `cargo run -p voice_server`; `cargo run -p voice_server -- --gen-signal-schema` writes `voice_signal_schema.json` (gitignored), the JSON Schema of every signalling frame, for the client's code generator.

Signalling is `voice_protocol::signal`: `identify` with the join token, then `ready` with the router's RTP capabilities and who is in the call; the client loads a mediasoup device, sends `setCapabilities`, creates and connects a send and a receive transport, and produces its microphone. The server creates a paused consumer on the client's receive transport for every producer of everyone else, present and future, announced with `newConsumer` and resumed by the client's `resumeConsumer`. Mute pauses the microphone producer on the server and deafen pauses every audio consumer (a shared screen stays visible to a deafened participant), both reported to the API server. A screen share is a second producer from the same send transport, `screen` for the picture and `screenAudio` for any sound the browser captured with it; mute never touches either. The `participantState` report carries `sharing_screen`, sent when a screen producer starts or closes as well as with mute and deafen changes, and `VoiceParticipant.sharingScreen` follows it so a client can mark who is sharing without being in the call. Speaking comes from mediasoup's audio level observer (300 ms interval, -50 dBvo). A call is a room: one router on one worker, created with the first participant and closed with the last, each step reported. `voice_protocol/examples/fake_voice_server.rs` sends any report or command by hand, which is how the API server's side is tested without media.

The client (`VoiceCall` in `client/packages/protocol/src/voice.ts`) pings every candidate's `/health`, tries them nearest first, and reports a server that fails to start its session. A client whose call ended with reason `serverLost` or `serverRemoved` rejoins the channel on its own after a random delay of at most one second, so the participants of a lost call do not all hit the API server in the same instant; `idle` and `empty` endings are not rejoined.

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
5. **Wire routes in `server/src/api/mod.rs`** — add `.routes(routes!(...))` calls in the `make_router` function.
6. **Add an ID type** (if needed) in `server/src/app/mod.rs` using the `id!` macro.
7. **Regenerate schemas** — run `cargo run -- --gen-openapi-schema` and commit the updated `openapi.yaml` and `event_schema.json`.

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

## The Two Insanities

As a crutch for very early stages development, the team has agreed to adhere to "The Two Insanities" which are intended to shorten how long it takes to get a usable product. Aspen cannot be used in production until the two insanities
are removed, but for now you may accept these flaws. The Two Insanities are:

1. Every user can take every action. We don't yet know what a good permissions model is going to be, so we are not implementing one at all.
2. Every user receives every event. We don't yet know how server events should be divided and routed, so we assume every user needs every event.

The long term goal is to eventually remove The Two Insanities, but you may use them for now to shorten how long it takes to get us to a point where clients can conceivably be built and used.

## Localization

Aspen uses [`rust-i18n`](https://crates.io/crates/rust-i18n) for internationalization of all client-facing strings. The `i18n!("locales")` macro is invoked in `server/src/main.rs`, and translation files live in `server/locales/`.

### Rules

- **Never hardcode English text** in API responses, validation errors, or any string returned to clients. Use the `t!()` macro instead.
- Log messages (`tracing::error!`, `tracing::warn!`, etc.) are **not** localized — they are developer-facing and should remain in English.
- Translation keys use **camelCase** (e.g., `inviteCodeLength`, `tryAgainLater`).
- The `t!()` macro returns `Cow<'static, str>`. The `app::Error::Validation` variant and all manually-defined API response error/reason fields accept `Cow<'static, str>` to match. Do not call `.to_string()` on `t!()` output.

### Adding a new client-facing string

1. Add the key and English text to `server/locales/en.yml`.
2. Use `t!("keyName")` in your Rust code, adding `use rust_i18n::t;` to the file's imports.
3. If the string contains interpolated values, use `t!("keyName", field = value)` and reference them in the YAML as `"... %{field} ..."`.

### File layout

- `server/locales/en.yml` — English translations (the default and only language for now). Additional locale files (e.g., `fr.yml`, `de.yml`) can be added alongside it when translation support expands.

## Git Policy

You are not to commit anything to `main` or push any branch to `origin`. If your work is deemed to be of sufficient quality, a human will send it on for you.