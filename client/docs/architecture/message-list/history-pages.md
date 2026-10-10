# History pages

| Constant | Value |
| --- | --- |
| `HISTORY_PAGE_SIZE` | 50 messages |
| `WINDOW_MAX_MESSAGES` | 300 messages in a channel's window |
| `LIVE_WINDOW_MAX_MESSAGES` | 600, for a window followed live |
| `KEPT_WINDOWS_MAX` | 30 channels' windows |
| `LOOSE_MESSAGES_MAX` | 1000 messages no window holds |

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

`RecordStore` (`packages/protocol/src/store.ts`) holds a bounded number of messages however long
the app runs.

- **A window drops from its far end** past `WINDOW_MAX_MESSAGES` as history pages in, and from its
  old end past `LIVE_WINDOW_MAX_MESSAGES` as messages arrive.
- **The windows of the `KEPT_WINDOWS_MAX` channels shown most recently are kept**, each followed
  live, so going back to one shows it at once. A window is shown while something subscribes to its
  topic (`messages:<channel>`), which a `MessageList` does for as long as it is drawn; one being
  shown is never dropped. A channel whose window was dropped reads its latest page when opened, as
  one never opened does.
- **Messages no window holds are loose**: those arriving in channels with no window at the latest,
  a dropped or replaced window's, and those read one at a time (a link's, a pin's, a saved
  message's, a thread's starter). The `LOOSE_MESSAGES_MAX` newest are kept. Past that the oldest
  go, with their reactions and annotations, except one something shows (a subscriber to
  `message:<id>`) and one that is unread and tags the reader, whose edit or deletion `AspenSync`
  must recognise to put the channel's unread tags right.
- **A link forgets a message that went.** The link's record said the message could be read; kept
  without the message it would read as a link to one deleted. Whoever shows the link reads it
  again (`loadLinks`).
- **A removed channel takes its loose messages with it**, as it takes its window.

So a message's record may be gone whenever nothing subscribes to it. What shows a message by id
outside a window reads it on demand (`useMessageOnDemand`, `loadMessage`).
