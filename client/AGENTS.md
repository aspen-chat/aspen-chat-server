# Aspen client — agent instructions

This directory contains the PySide6 desktop client. The rules below are load-bearing; if you're tempted to change any of them, stop and get explicit human approval first.

## Non-negotiable: every `AspenApiClient` call runs on a worker thread

Qt's event loop is single-threaded. Anything that blocks it — an HTTP round-trip, a DNS lookup, a slow TLS handshake — freezes every widget in the window: keystrokes pile up behind the freeze, scroll events are dropped, repaints stall, and users perceive the client as skipping characters or "eating" clicks. We had exactly this bug in `_send_clicked` because it was calling `self._api.send_message(...)` directly on the GUI thread. Do not reintroduce it anywhere else.

The one and only way to invoke a blocking method on `AspenApiClient` from inside `ChatWindow` (or any other GUI object) is through `AsyncApiCaller.submit`:

```python
self._async_api.submit(
    lambda api: api.read_community_channels(community_id),
    on_success=lambda channels: self._on_community_channels_loaded(community_id, channels),
    on_error=lambda exc: self._status_label.setText(f"Failed to load channels: {exc}"),
)
```

Rules that follow from this:

- **Never** call `self._api.<anything>` directly from an event handler, a paint path, a row insert, a scroll callback, `eventFilter`, a slot connected to `clicked`, or any other GUI-thread code. The only exception is `self._api.close()` in `closeEvent`, which is a local teardown that touches no network.
- Every new feature that needs to talk to the server must thread a worker submission through the call site instead of synchronously awaiting a result. If you catch yourself "just this once" inlining a sync call because the call "is usually fast", stop — `usually` is not a guarantee, and the frozen-composer bug shipped exactly because the send "is usually fast".
- Render hot-paths (row inserts, avatar lookups, profile name rendering) must never block on the network either. The established pattern is: render a fallback synchronously, kick off an async fetch, and patch the affected widgets in the `on_success` callback. See `_user_avatar_pixmap` / `_on_user_icon_loaded` and `_resolve_author_profiles` / `_on_user_profile_loaded` for the reference shape.
- `on_success` callbacks always run on the GUI thread (that is the whole point of `AsyncApiCaller`), so widget and `ClientState` mutations inside them are safe. Conversely, the operation lambda runs on a worker thread — do not touch widgets or `ClientState` from inside it.
- When a fetch's result depends on UI state that can change in the meantime (e.g. "load channels for the currently selected community"), capture the relevant id in the closure and have `on_success` bail if it no longer matches the current selection. Otherwise a slow response will clobber the newer selection.
- Prefer dedupe sets (see `_pending_user_profile_fetches`, `_pending_user_icon_fetches`, `_pending_community_icon_fetches`, `_pending_fetches` for the paged-read variant) over letting the same fetch queue up repeatedly. A scrollbar dwelling at an edge or a row being re-rendered should not generate hundreds of duplicate requests.

If you need a new concurrency primitive for the client, extend `AsyncApiCaller` rather than introducing a second worker class. DRY matters here precisely because the worker is the only thing standing between the user and a frozen UI.

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

### 2. Paginated reads via `AsyncApiCaller`

- All paginated reads (`initial`, `older`, `newer`) are dispatched by `_dispatch_message_fetch` in `ui.py`, which submits the blocking `read_channel_messages` call through `AsyncApiCaller` and receives the result on the GUI thread. This is a specific instance of the general "every API call goes through the worker" rule above.
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

## What's explicitly out of scope

The following are tempting "improvements" that you should not make without a fresh, human-approved plan:

- Switching from `QListWidget` to `QListView` + a custom model. Not needed while `MESSAGE_WINDOW_CAP` keeps widget count bounded. If the cap grows past a few thousand, revisit.
- Unbounded caching of message history "because we already fetched it". The whole point of eviction is that memory is bounded across many visited channels.
- Eagerly prefetching multiple pages on channel switch. One `initial` fetch per switch is enough; the scroll handler handles everything else.
- Inlining synchronous HTTP calls anywhere on the GUI thread, including during channel switch, login, event handling, row rendering, icon lookup, or profile resolution. See the "every `AspenApiClient` call runs on a worker thread" rule above — that rule is the canonical statement of this constraint.

## Code-generation boundary

`src/aspen_client/generated/` is produced from the server's `openapi.yaml` and `event_schema.json`. Do not edit files under that directory by hand. If types need to change, change the server, regenerate the schemas, then regenerate the client models.

## Repository-wide rules

The repository root `AGENTS.md` still applies here (tech stack, two insanities, localization, git policy, etc.). Nothing in this file overrides it.
