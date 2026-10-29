# History pages

| Constant                   | Value                              |
| -------------------------- | ---------------------------------- |
| `HISTORY_PAGE_SIZE`        | 50 messages                        |
| `WINDOW_MAX_MESSAGES`      | 300 messages in a channel's window |
| `LIVE_WINDOW_MAX_MESSAGES` | 600, for a window followed live    |
| `KEPT_WINDOWS_MAX`         | 30 channels' windows               |
| `LOOSE_MESSAGES_MAX`       | 1000 messages no window holds      |

- `useHistoryPaging` (`historyPaging.ts`) decides when a page is read. It is asked whenever the view
  moves or is placed afresh.
- A page renders as a transition, in slices between which the finger is heard.
- At fifty messages, committing a page's rows stays under a long task's 50ms. A hundred took over
  120ms.
- `MessageItem` is memoized, so a change to the list renders only the rows it changes.

**Keep `MessageItem` memoized.** An unmemoized row makes every change to the list (a loading line's
included) render every row through its Markdown again, synchronously, for half a second at a time.

When a deleted message's space closes, `useShownWindow` holds the change to the window and shows it
as a transition.

## What the store keeps

`RecordStore` (`packages/protocol/src/store.ts`) holds a bounded number of messages however long the
app runs.

- **A window drops from its far end** past `WINDOW_MAX_MESSAGES` as history pages in, and from its
  old end past `LIVE_WINDOW_MAX_MESSAGES` as messages arrive.
- **The windows of the `KEPT_WINDOWS_MAX` channels shown most recently are kept**, each followed
  live, so going back to one shows it at once. A window is shown while something subscribes to its
  topic (`messages:<channel>`), which a `MessageList` does for as long as it is drawn; one being
  shown is never dropped. A channel whose window was dropped reads its latest page when opened, as
  one never opened does.
- **Messages no window holds are loose**: those arriving in channels with no window at the latest, a
  dropped or replaced window's, and those read one at a time (a link's, a pin's, a saved message's,
  a thread's starter). The `LOOSE_MESSAGES_MAX` newest are kept. Past that the oldest go, with their
  reactions and annotations, except one something shows (a subscriber to `message:<id>`) and one
  that is unread and tags the reader, whose edit or deletion `AspenSync` must recognise to put the
  channel's unread tags right.
- **A link forgets a message that went.** The link's record said the message could be read; kept
  without the message it would read as a link to one deleted. Whoever shows the link reads it again
  (`loadLinks`).
- **A removed channel takes its loose messages with it**, as it takes its window.

So a message's record may be gone whenever nothing subscribes to it. What shows a message by id
outside a window reads it on demand (`useMessageOnDemand`, `loadMessage`).

## Reads that land after events

The server publishes an event before it answers the request that made the change, so a REST response
can carry a record older than an event the store has already applied: a page of history read while
someone edits or deletes a message on it, a member read while someone is renamed. The rule for
writes, that a response never overwrites a record the stream already holds, holds for reads too: **a
read never overwrites a record the stream changed since the read began, and never brings back a
message the stream deleted since.**

- `AspenSync` notes the store's clock before every read is sent and passes it as `since`
  (`ReadOptions`) to whatever installs the answer: `ingest`, `replaceWindow`, `prependWindow`,
  `appendWindow`, `setReactions`, `noteMemberships`, `applyStatuses`. A read with no `since`, the
  bootstrap, is installed whole; the events that arrive during it are held back and applied after.
- The store remembers when an event last changed each record (`message:<id>`, `user:<id>`,
  `channel:<id>`, `community:<id>`, `category:<id>`, `poll:<id>`, `reactions:<id>`,
  `member:<community>:<user>`, `read:<channel>`), and when it deleted each message. Only events
  touch them (`applyEvent`, the cascades it runs, a `presence` frame through `noteStatuses`, the
  caller's own chosen status), never reads, so a read never blocks another. A record touched at or
  after `since` keeps the stream's version; a message deleted at or after `since` is left out of the
  window and the store.
- A window read that reaches the latest also keeps every held message of the channel newer than the
  read's newest: those can only have come by event or the caller's own send since the read's
  snapshot, and dropping them would leave a hole no later read fills, since a window at the latest
  reads no further pages.
- Read states a read sideloads merge forward (`putReadState`: a position never moves back) and are
  skipped when the stream moved the channel's state since; a read `AspenSync` makes because of an
  event (a channel's state after its newest message was deleted) carries no `since`, since its
  answer is the recount the event asked for.
- A touch is remembered for `TOUCH_MEMORY_MS` (two minutes, longer than any request stays in
  flight), and the oldest are forgotten past `TOUCHES_MAX`; a read that outlasts that is installed
  as if nothing had changed.

What this does not cover: an event replayed _after_ a read's response that already reflected it (a
`react` for a fifth-or-later reactor landing after a window read that counted them) counts twice
until the message's reactions are read again, since neither side carries the feed's sequence. The
replay a connection starts with cannot land so, since window reads wait for it (below); only a
server that writes no `caughtUp` leaves the chance. Annotations and attachments have no touches, and
the attachment record keeps a preview a read lacks on its own (`#putAttachment`).

## The replay a connection starts with

Every connection of the event stream begins with a replay: the stream's last minute on a first
connection, what was missed on a resumed one. The server's `ready` says what the replay is worth,
and `AspenSync` decides from it whether the cache can be trusted:

- `replayComplete: false` says the server left out events it could not judge (those of a community
  from before a change of the caller's roles there, say). The cache is read again (`resyncing`),
  resumed or not.
- `replayedSince` says when the replayed window begins, when it is shorter than the usual minute
  (events let go of early, or a server that started reading the stream less than a minute ago). A
  bootstrap that began before it, less `REPLAY_CLOCK_SLACK_MS` for the two clocks disagreeing, may
  have missed events between, so the cache is read again. A server that does not say leaves the
  fallback: a first connection's bootstrap that took longer than `BOOTSTRAP_STALE_AFTER_MS` is read
  again.
- `catchUpFollows: true` says the server ends the replay with a `caughtUp` frame. The sync stays
  `connecting` until it, and window reads asked for meanwhile wait for it (`#whenCaughtUp`), so the
  replay is applied before any read is sent and can never land over a read's answer. A server that
  writes none is live from `ready`, as before there was one.

A `ready` that arrives while a bootstrap is in flight (a reconnect during a resync) and says the
replay may lack events has the bootstrap done again once it lands, since the one in flight began
before the connection. The decision is `#replayMayLack`; the stream parses the frame (`EventStream`,
`ReadyInfo`, `onCaughtUp`) and ignores frames of types it does not know.
