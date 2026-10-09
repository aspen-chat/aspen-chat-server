# The message list: design notes

Rationale behind the [message list](index.md) pages.

## Why the list scrolls itself on iOS

See [who scrolls the list](scrolling.md).

- **On iOS the list scrolls itself, for one reason.** A box the browser scrolls for the user is the
  one thing that cannot be kept still there. iOS scrolls such a box in a process of its own, and
  places it from where a pan began plus the finger's travel. A position set from the page while a
  finger drags or a fling runs (by WebKit's anchoring or by script) is applied and then overridden by
  the pan's next update. The view lands wherever the change put it, a page's height away.
- **A box only scripts scroll has no such gesture.** Nothing overrides what the list sets. Scripts
  still scroll it, so focus, find-in-page, assistive technology, and Playwright bring things into
  view as they always did.
- **The pan's override was found by the iOS simulator test** `testReadingBackQuicklyNeverJumps` with
  `scrollDiagnostics.ts` (see [tests](tests.md)).

## Why the browser scrolls everywhere else

- **The browser's scrolling is the better one where it can be kept still.** It runs on the
  compositor, keeps moving while the main thread is busy, draws the platform's indicator and
  overscroll, and gives VoiceOver its three-finger scroll, which a box that hides its overflow does
  not.
- **The conditions for giving it back on iOS too**, and the experiment that proves them, are in
  `MessageList.tsx`'s header. Nothing outside the list may assume either way.

## Dragging and events

- **Rows take no pointer once a finger has moved `DRAG_SLOP_PX`.** Lifting it over a picture or a
  button is then not a press, as it would not be under a pan the browser made.
- **The list takes only events from the list itself.** React passes events up through what a row
  opens in a layer of its own over the page (the actions' sheet, the emoji picker's, a dialog) as
  though it were in the row. A finger scrolling a picker there would otherwise drag the list behind
  it.

## Keeping still

See [keeping the view still](keeping-still.md).

- **The browser's scroll anchoring is off on every platform.** The list keeps its own view still.
- **A linked message is held by its middle.** A jump lands on it, and it stays centred whatever loads
  around it or inside it.
- **The list's own moves catch up first (`catchUp`).** They run before the frame's resize observers
  are told, and would otherwise note the row where the change put it.
- **Pinning is judged against the content's height measured afresh.** The browser also scrolls the
  box when what lies beneath it shrinks, and the bottom it was at moves up (the message box losing
  its files as a message is sent).

## Pages

See [history pages](history-pages.md).

- **A page is fifty messages.** Committing its rows stays under a long task's 50ms, where a hundred
  took over 120ms.
- **A page renders as a transition, in slices.** The finger is heard between slices.
- **`MessageItem` is memoized.** An unmemoized row makes every change to the list, a loading line's
  included, render every row through its Markdown again, synchronously, for half a second at a time.

## Keyboard

See [moving between messages by keyboard](keyboard.md).

- **A row asks for the stop with `useRowStop`.** Moving re-renders only the rows the stop leaves and
  reaches.
