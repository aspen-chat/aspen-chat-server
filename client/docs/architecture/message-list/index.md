# The message list's scrolling

How the message list scrolls, keeps what is in view still as things change around it, reads history
a page at a time, moves between messages by keyboard, and reads out arrivals.

## Pages

- [Who scrolls the list](scrolling.md): the list scrolls itself on iOS and iPadOS, and the browser
  scrolls it everywhere else.
- [Keeping the view still](keeping-still.md): the still row, catching up, linked messages, pictures'
  room, and pinning to the bottom.
- [History pages](history-pages.md): page size, rendering pages as transitions, memoized rows, and
  what the store keeps of windows and of messages outside them.
- [Moving between messages by keyboard](keyboard.md): the list's one tab stop and row keys.
- [What a row holds](memory.md): actions built only for the rows in use, overlays built once
  opened, and what a row costs.
- [Reading out arrivals](announcing-arrivals.md): announcing new messages to screen readers.
- [Tests](tests.md): the Playwright history tests and the iOS simulator test.
- [Design notes](design-notes.md): why the list works this way.

## Parts

The code is in parts, each with one concern. Files are in `src/features/messages/`.

| Part | Where | Concern |
| --- | --- | --- |
| `MessageList` | `MessageList.tsx` | Renders the rows and joins the rest. |
| `ListScroller` | `listScroller.ts` | A plain class, outside React's renders, that holds the position and every step that sets it: noting and holding the still row, the pin to the bottom, going to a linked message, the observers of the rows and the viewport, and on iOS the fingers, wheel, keys, coasting, spring, and indicator. |
| `ListScroller`'s hooks | `listScroller.ts` | It has two hooks. `useListScroller` keeps one `ListScroller` for the life of a message list; a different channel starts pinned to the bottom. `useListPosition` gives it what each render knows before it places the view, so its handlers, observers, and running flings always act on the latest render's window and callbacks. |
| Scroll arithmetic | `scrollPhysics.ts` | The arithmetic of the list's own scrolling on iOS. |
| `useHistoryPaging` | `historyPaging.ts` | Decides when a page is read. Asked whenever the view moves or is placed afresh. |
| `useShownWindow` | `shownWindow.ts` | Holds a change to the window while a deleted message's space closes, and shows it as a transition. |
| `useNewMessagesLine` | `newMessagesLine.ts` | Places the "New Messages" line. |
| `useReadMarking` | `useReadMarking.ts` | Marks what is seen as read. |
| `useKeepStill` | `keepStill.ts` | Lets a picture tell the list it arrived. |
| `MessageRows` | `messageRows.ts` | Keyboard focus between rows, and which rows are engaged. |
| `useAnnounceArrivals` | `announceArrivals.ts` | Reads out arriving messages. |
| Diagnostics | `scrollDiagnostics.ts`, `ScrollDiagnosticsPanel` | Records what the list does with its position, in a `VITE_SCROLL_DEBUG=1` build. |
