# History pages

| Constant | Value |
| --- | --- |
| `HISTORY_PAGE_SIZE` | 50 messages |

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
