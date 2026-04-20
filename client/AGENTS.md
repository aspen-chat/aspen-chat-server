# Aspen client — agent instructions

This directory contains the PySide6 desktop client. The rules below are load-bearing; if you're tempted to change any of them, stop and get explicit human approval first.

## Non-negotiable: every `AspenApiClient` call is awaited from a `TaskSpawner`-spawned coroutine

Qt's event loop is single-threaded. Anything that blocks it — an HTTP round-trip, a DNS lookup, a slow TLS handshake — freezes every widget in the window: keystrokes pile up behind the freeze, scroll events are dropped, repaints stall, and users perceive the client as skipping characters or "eating" clicks. We had exactly this bug in `_send_clicked` because it was calling `self._api.send_message(...)` synchronously on the GUI thread. Do not reintroduce it anywhere else.

`AspenApiClient` is built on `httpx.AsyncClient`; every method (`login`, `read_community_channels`, `send_message`, `read_icon_bytes`, …) is `async def`. The asyncio loop runs on top of the Qt event loop via `qasync` (started in `main.py`; `qasync.QEventLoop` is what httpx/anyio actually run network I/O against — `PySide6.QtAsyncio` is a stub that doesn't implement `create_connection`/`getaddrinfo` and must not be reintroduced). Coroutines and Qt slots both execute on the GUI thread, and an `await` point on the API client yields control back to the event loop while the network round-trip is in flight — the same way the previous worker-thread hand-off did, just without the thread.

GUI slots stay synchronous (Qt requires it). Anything that needs to `await` is dispatched through `self._tasks.run(coro, on_success=..., on_failure=...)`. The helper schedules the coroutine and routes the awaited result to the success/failure callback on the GUI thread; the per-call `try`/`except`/dispatch boilerplate that used to live in a hand-written `async def _do_xxx` helper now lives once, inside `TaskSpawner.run`. The canonical shape is:

```python
def _community_changed(self) -> None:
    ...
    preferred_channel_id = self._current_channel_id
    self._tasks.run(
        self._api.read_community_channels(community_id),
        on_success=lambda channels: self._on_community_channels_loaded(
            community_id, channels, preferred_channel_id
        ),
        on_failure=lambda exc: self._status_label.setText(f"Failed to load channels: {exc}"),
    )
```

When a coroutine needs more setup than a single API call (paged-read selection of `before` / `after` / `around`, e.g. `MessagePane._dispatch_message_fetch`), build the coroutine first and then hand it to `self._tasks.run` — the helper just needs *a* coroutine, not specifically a method on `AspenApiClient`. The lower-level `self._tasks.spawn(coro)` is still available for fire-and-forget background loops with their own internal exception handling (the WebSocket reader in `EventStreamClient._run` is the only current example).

Rules that follow from this:

- **Never** call `self._api.<anything>` from a synchronous GUI slot, a paint path, a row insert, a scroll callback, `eventFilter`, a slot connected to `clicked`, or any other non-async code. Every API method is a coroutine; calling one from sync code without `await` returns an un-awaited coroutine and emits a runtime warning rather than performing the request. The only sanctioned exception is `await self._api.aclose()` inside `_async_shutdown`, which is part of the local teardown sequence.
- Every new feature that needs to talk to the server must dispatch through `self._tasks.run(...)` (or, for fire-and-forget loops, `self._tasks.spawn(...)`). If you catch yourself "just this once" calling a coroutine via `asyncio.run` or `loop.run_until_complete` from inside a slot because the call "is usually fast", stop — both of those nest event loops, and `usually` is not a guarantee.
- Render hot-paths (row inserts, avatar lookups, profile name rendering) must never `await` synchronously either. The established pattern is: render a fallback synchronously inside the slot, kick off the fetch via the relevant cache (`self._icons.user_avatar_pixmap` or `self._users.request_profiles`), and patch the affected widgets in the cache's "ready" callback once the bytes land. `IconCache` (`src/aspen_client/icons.py`) and `UserDirectory` (`src/aspen_client/user_directory.py`) own this state for the icon and profile/presence sides respectively; both expose the same shape (synchronous getter for the render hot-path + async background fetch + ready-callback) and `ChatWindow` is the consumer of both.
- Code after an `await` runs on the GUI thread (that is the whole point of running asyncio on top of Qt's event loop), so widget and `ClientState` mutations inside the `on_success` / `on_failure` callback (or any other coroutine scheduled through `TaskSpawner`) are safe. There is no longer a "worker thread" you have to keep widget code out of — including the WebSocket reader, which is now an asyncio coroutine running on the same loop.
- When a fetch's result depends on UI state that can change in the meantime (e.g. "load channels for the currently selected community"), capture the relevant id in the closure of the `on_success` lambda and have the handler bail if it no longer matches the current selection. Otherwise a slow response will clobber the newer selection. (See `_on_community_channels_loaded` and `_on_message_page_loaded` for the existing equality-check guards.)
- Prefer dedupe sets over letting the same fetch queue up repeatedly. A scrollbar dwelling at an edge or a row being re-rendered should not generate hundreds of duplicate requests. The dedupe sets live with the cache that owns the fetch: `UserDirectory._pending_profile_fetches` for user profiles, `IconCache._pending_user_icon_fetches` / `IconCache._pending_community_icon_fetches` for avatars, `MessagePane._pending_fetches` for paged reads. Releasing the dedupe slot on the error path is the `on_failure` lambda's job (see `UserDirectory.request_profiles` for the reference shape).

If you need a new concurrency primitive for the client, extend `TaskSpawner` rather than introducing a second scheduler. DRY matters here precisely because a single, well-understood spawner is the only thing standing between the user and a frozen UI — and between a clean shutdown and orphaned coroutines outliving the window. (`AsyncApiCaller` from earlier revisions of this file no longer exists; `TaskSpawner` is its asyncio-native successor.)

## Non-negotiable: the event-stream reconnect contract

`EventStreamClient` (`src/aspen_client/event_client.py`) is the WebSocket reader for live server events. It runs as an `asyncio` coroutine on the qasync loop (started via `EventStreamClient.start(self._tasks)` from `_on_login_succeeded`); reconnects are managed by a `tenacity.AsyncRetrying` schedule. The UI integrates via four Qt signals — `event_received(dict)`, `connection_lost(str)`, `connected()`, and `state_resync_required()` — and a single state-wipe helper, `ChatWindow._reset_client_state()`. The whole design is anchored to one server-side fact: the NATS JetStream consumer in `server/src/api/event_stream.rs` is created with `DeliverPolicy::ByStartTime { start_time: now - MAX_EVENT_AGE }` where `MAX_EVENT_AGE = 60s`. So the server replays the last 60 seconds of events to any reconnecting client, no client-supplied cursor, no resume token.

The client reconnect policy is calibrated to that server window:

- The first reconnect attempt of every outage episode is **immediate** (no delay; tenacity does not wait before attempt #1). Subsequent failures back off `0.5s → 1s → 2s → 4s → 5s` (capped via `wait_exponential(multiplier=0.5, max=5.0)`). This continues indefinitely (`stop=stop_never`) until `stop()` — there is no failure-count ceiling, because the user expects the client to come back if connectivity returns hours later.
- The grace deadline is **45 seconds** (`_RECONNECT_GRACE_SECONDS`), measured from the start of the outage. It is a *deadline*, not an *interval*: we keep retrying past it. If a connect succeeds within the deadline, the server's 60s replay buffer is guaranteed to cover the gap and in-memory state stays valid. If the connect succeeds after the deadline, the replay can no longer be assumed to cover the whole gap, and the client must treat its caches as stale.
- These two numbers (45s grace, 60s server replay) are coupled. If `MAX_EVENT_AGE` ever changes server-side, `_RECONNECT_GRACE_SECONDS` must move with it, keeping a safety margin (currently 15s) to absorb wall-clock skew between the disconnect timestamp and the JetStream window boundary. Don't tune one without the other.

Signal contract — keep these exact semantics:

- `connection_lost(str)` is emitted **exactly once per outage episode**, at its start (initial-connect failure, or post-connect drop). Not on each retry. The UI surfaces it as a single status update; if it fired per-retry the status bar would flicker. The string is informational and currently surfaced as a tooltip, not as the visible text.
- `connected()` is emitted on every successful connect, including reconnects. The UI uses it to clear the "Disconnected" status; it deliberately only overwrites status text that begins with "Disconnected" so it doesn't clobber unrelated user-action statuses (e.g. "Message sent") that may have been written during steady-state operation.
- `state_resync_required()` is emitted **immediately before** `connected()` on a reconnect that crossed the 45s deadline. Qt delivers queued cross-thread signals in emission order, so the UI's resync slot runs first and stages the cache wipe + re-bootstrap before the connected slot writes "Connected". If you ever change the order, you must also change the UI's interaction between these two slots, otherwise the transient "Reconnected — refreshing state…" status will be lost.

UI side: `ChatWindow._on_state_resync_required` calls `_reset_client_state()` and then re-runs the initial communities load via `_dispatch_initial_communities_load(preferred_community_id=...)`. Two invariants here matter:

- `_reset_client_state()` wipes every cache derived from server state (`_state`, the `IconCache`/`UserDirectory` caches, the `KeyedListWidget`-managed lists, the user-row entry map, the message pane's window, and any `_pending_*` dedupe sets owned by the caches). It must keep being a 1:1 mirror of the server-derived field initialisation in `__init__`; any new server-derived cache you add elsewhere needs a `clear()` call here too, or the resync will leave stale references. It deliberately does **not** touch `_current_community_id` / `_current_channel_id` (the rebootstrap reselects them) and does **not** clear the visible widgets directly — the `KeyedListWidget.replace_all` calls that run as the bootstrap completes do the on-screen replacement, so the user sees the previous UI continuously until it's overwritten pane by pane (the "minimal" UX choice).
- The rebootstrap deliberately does not cancel in-flight `TaskSpawner` tasks left over from before the disconnect. Those tasks complete against the freshly-emptied state, miss most of their lookups, and effectively no-op; the rebootstrap then overwrites everything. Cancelling them up-front would race with `asyncio` task teardown and isn't worth the complexity.

`_handle_event` and the per-type event handlers must remain robust to events arriving against an empty `_state` / empty row-index dicts (they currently bail on `not in` lookups and missing dict entries). The reconnect path relies on this because events arriving on the new WebSocket can race the rebootstrap.

## Non-negotiable: the sliding-window message architecture

Aspen channels are expected to live forever and accumulate effectively unbounded histories. To keep the client responsive regardless of how much history a channel has, message history is rendered through a **bidirectional sliding window** per channel. **Do not remove, weaken, or work around this architecture.**

The architecture has four cooperating pieces. Each piece is load-bearing; breaking any one of them silently re-introduces the "slow after a few dozen messages" / "OOM after a few days" regressions this design was built to prevent.

### 1. `ChannelMessageWindow` in `src/aspen_client/state.py`

- `ClientState.channel_windows: dict[str, ChannelMessageWindow]` is the single source of truth for which messages exist on the client for a given channel.
- `ChannelMessageWindow` holds `ordered_ids` (ascending UUID v7, which is also ascending time), `has_older`, and `has_newer`.
- `has_newer` **must** stay True whenever the window is not sitting on the server tip (either because the user scrolled back and we evicted the newest, or because we loaded a middle slice). Live WebSocket events for a channel in that state **must** be dropped by `upsert_message`; do not try to "helpfully" append them — that would produce a non-contiguous slice and break pagination invariants downstream.
- `MESSAGE_WINDOW_CAP` (currently 500) is the hard ceiling on ids retained per channel. `upsert_message` enforces it on live appends; `_on_message_page_loaded` in the UI enforces it on paged reads. Both paths must remain; removing either re-introduces unbounded growth.
- When ids are evicted from a window, the corresponding records **must** also be dropped from `ClientState.messages`. Keeping them around defeats the memory bound.
- `merge_channel_page` is direction-agnostic and is the only correct way to fold a paged read result into a window. Do not add special-case prepend/append helpers unless you have a concrete reason the direction-agnostic version can't serve.

### 2. Paginated reads via `TaskSpawner`

- All paginated reads (`initial`, `older`, `newer`) are dispatched by `MessagePane._dispatch_message_fetch` in `ui_messages.py`, which builds a `read_channel_messages` coroutine and hands it to `self._tasks.run(...)`; the success callback routes the result back to `_on_message_page_loaded` on the GUI thread (which is where the asyncio loop runs anyway). This is a specific instance of the general "every API call is awaited from a `TaskSpawner`-spawned coroutine" rule above.
- `read_channel_messages` accepts at most one of `before` / `after` / `around`. Keep that invariant; it maps directly to the server's `ChannelViewDescription` discriminated union.
- Count is clamped to `[1, 200]` to match the server's `MAX_MESSAGES_QUERIED`. Don't raise it without a matching server-side change.

### 3. Scroll-driven paging in `src/aspen_client/ui.py`

- `_on_messages_scrolled` watches the vertical scrollbar and triggers `older` / `newer` fetches when near the edges of the viewport. This is what makes the scroll feel infinite; deleting it reverts the client to a single static page.
- `_pending_fetches` prevents piling up concurrent identical requests while the bar dwells at an edge. Don't remove this dedupe; without it a slow connection can queue hundreds of redundant fetches.
- `_capture_top_anchor` / `_restore_top_anchor` pin the user's visual scroll position across an older-direction prepend. If this is broken, scrolling up causes the content to jump, which is what every other chat client in 2026 has gotten right and users expect us to as well.
- The loading sentinel rows (`_loading_header_item`, `_loading_footer_item`) are the user-visible signal that a fetch is in flight. Keep them.

### 4. The "Jump to latest" affordance

- The floating button is visible **if and only if** `window.has_newer is True`. It's how users escape from a scrolled-back state back to the tip. Clicking it clears the window and dispatches a fresh `initial` fetch.
- `_send_clicked` detects the "user is scrolled back and just sent a message" case and triggers the same jump-to-latest reset. Do not try to append the sent message to a mid-history window; that violates the contiguous-slice invariant (see §1).

## Non-negotiable: link previews are server-authoritative

Link previews for message bodies are generated **entirely on the server**. The server parses the message markdown, extracts URLs, fetches each target asynchronously, stores preview metadata in the `message_link_preview` table, uploads preview images into the shared `MediaStore` under `link-preview-images/`, and publishes the authoritative preview list on a dedicated `messageLinkPreviewsReady` event. The client does **not** scrape URLs out of message bodies, does **not** make its own HTTP requests to third-party hosts, and does **not** invent its own caps on previews-per-message — the server owns all three concerns and does not want the client second-guessing any of them.

Concretely, on the client:

- `Message.link_previews: list[LinkPreview]` (in `src/aspen_client/types.py`) is the ground truth for which preview cards a row renders. REST responses carry it inline on the message; later revisions arrive via `messageLinkPreviewsReady`, which `ChatWindow._handle_message_link_previews_ready` routes into `ClientState.apply_link_previews_ready` and then through `MessagePane.handle_link_previews_ready`. A row displayed against a `Message` whose `link_previews` is `[]` shows no preview row, even if the body contains URLs — that means the server deliberately skipped them (crawl refused by robots, fetch failed, content-type not HTML, etc.), and the client must not override that decision.
- `_render_markdown_to_html` in `src/aspen_client/ui_messages.py` is **only** responsible for turning the body into sanitised rich text (see `MarkdownNoHTML` + `_disarm_misleading_links`). It does not extract URLs, does not return a URL list, and must not grow one; the preview card stack is built entirely from `message.link_previews` in `_build_preview_cards` / `_reset_preview_cards`.
- Preview **thumbnail images** are the one remaining network hop the client makes, and that hop goes to the Aspen server via `GET /link-preview-image` (the `AspenApiClient.read_link_preview_image` method). `LinkPreviewImageCache` in `src/aspen_client/link_preview.py` mirrors the `IconCache` pattern (synchronous `get_pixmap` / `has_settled` for the render hot-path, async background fetch through `TaskSpawner`, `on_image_ready` callback, dedupe of in-flight ids). `MessagePane._image_subscribers` maps `image_id → set(message_id)` so a landed thumbnail only repaints the rows that actually asked for it. When a row is evicted (`_evict_ui_rows_not_in_window`, `_drop_preview_subscriptions`) or its preview stack is replaced (`_reset_preview_cards`, `handle_link_previews_ready`), the subscription entries are pruned in the same pass — otherwise a thumbnail arriving after eviction would try to repaint a widget Qt has already freed.
- The `messageLinkPreviewsReady` event replaces the cached message's `link_previews` **wholesale**; the server always publishes the full authoritative list (even an empty one on an edit that stripped every URL). Do not try to diff against the previous list to preserve "existing" cards — the server has already reasoned about which URLs survived the edit.

Out-of-scope "improvements" in this area, for the same reason as the other non-negotiables:

- Re-scanning `message.content` for URLs on the client. This puts the client and server out of sync on what counts as a preview-worthy URL and re-introduces the network fan-out the server migration was built to eliminate.
- Fetching third-party hosts directly from the client (Open Graph metadata, favicons, preview images). All outbound link traffic must originate from the server so the user's IP is never exposed to URL targets by merely viewing a message.
- A client-side cap on previews per message, or hiding preview cards whose metadata "looks empty". The server already applies the cap and already filters out previews it considers unusable; anything that reaches `Message.link_previews` is meant to render.

## What's explicitly out of scope

The following are tempting "improvements" that you should not make without a fresh, human-approved plan:

- Switching from `QListWidget` to `QListView` + a custom model. Not needed while `MESSAGE_WINDOW_CAP` keeps widget count bounded. If the cap grows past a few thousand, revisit.
- Unbounded caching of message history "because we already fetched it". The whole point of eviction is that memory is bounded across many visited channels.
- Eagerly prefetching multiple pages on channel switch. One `initial` fetch per switch is enough; the scroll handler handles everything else.
- Awaiting `AspenApiClient` methods anywhere outside a coroutine scheduled via `self._tasks.run(...)` or `self._tasks.spawn(...)`, or "just calling" a coroutine from a sync slot without scheduling it. See the "every `AspenApiClient` call is awaited from a `TaskSpawner`-spawned coroutine" rule above — that rule is the canonical statement of this constraint and applies during channel switch, login, event handling, row rendering, icon lookup, and profile resolution.

## Code-generation boundary

`src/aspen_client/generated/` is produced from the server's `openapi.yaml` and `event_schema.json`. Do not edit files under that directory by hand. If types need to change, change the server, regenerate the schemas, then regenerate the client models.

## Repository-wide rules

The repository root `AGENTS.md` still applies here (tech stack, two insanities, localization, git policy, etc.). Nothing in this file overrides it.
