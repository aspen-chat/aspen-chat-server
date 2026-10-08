# The message list's scrolling

On iOS and iPadOS the list scrolls itself (`OWNS_SCROLLING` in `listScroller.ts`, whose
`ListScroller` holds the list's position and everything that sets it, with the arithmetic in
`scrollPhysics.ts`): its box hides its overflow, so no finger, wheel, or key
scrolls it, and the list takes those itself and sets the box's scroll position from them,
with its own coasting, spring at the ends, and indicator. A box the browser scrolls for the
user is the one thing that cannot be kept still there: iOS scrolls such a box in a process of
its own and places it from where a pan began plus the finger's travel, so a position set
from the page while a finger drags or a fling runs, by WebKit's anchoring or by script, is
applied and then overridden by the pan's next update, and the view lands wherever the change
put it, a page's height away. A box only scripts scroll has no such gesture, and nothing
overrides what the list sets; scripts still scroll it, so focus, find-in-page, assistive
technology, and Playwright bring things into view as they always did. Everywhere else the
browser scrolls the box, on its compositor, with its own indicator, overscroll, and assistive
gestures, and honours a position the list sets at any moment; the browser's scroll
anchoring is off on the list on every platform, since the list keeps its own view still,
and what it does with the position is the same either way. The Chromium history tests run
both ways, Chromium standing in for iOS by claiming the property the list knows it by. Where the list scrolls itself, once a finger has moved `DRAG_SLOP_PX` it is dragging,
and the rows take no pointer until it lifts, so lifting it over a picture or a button is not
a press, as it would not be under a pan the browser made. It takes only what happens in the list itself: React passes events up through what a row
opens in a layer of its own over the page (the actions' sheet, the emoji picker's, a dialog)
as though it were in the row, and a finger scrolling a picker there would otherwise drag the
list behind it. What is in view never moves when something changes around it: the list notes a row
and where it stands in the content (`still`), which scrolling does not change, and after
every change, a page's arrival at commit, a picture's arrival told by the picture itself in
the same task (`useKeepStill`), or any change of the rows' size seen by a `ResizeObserver`
before the frame is painted, moves the position by what that row has moved; and every move
of the list's own, a finger's or a fling's frame, first takes in whatever moved that row in
the content since it was noted (`catchUp`), since those run before the frame's resize
observers are told and would otherwise note the row where the change put it. The row is the
topmost in view, or, while a linked message is shown, that message, held by its middle so a
jump lands on it and stays centred whatever loads around it or inside it; the reader's first
scroll drops the link and the hold returns to the topmost row. The list scrolls itself on
iOS for that one reason, and the browser's scrolling is the better one otherwise (it runs on
the compositor, keeps moving while the main thread is busy, draws the platform's indicator
and overscroll, and gives VoiceOver its three-finger scroll, which a box that hides its
overflow does not): the conditions for giving it back there too, and the experiment that
proves them, are in `MessageList.tsx`'s header, and nothing outside the list may assume
either way. A page renders as a transition,
in slices between which the finger is heard, and is fifty messages (`HISTORY_PAGE_SIZE`),
at which committing its rows stays under a long task's 50ms where a hundred took over 120ms;
`MessageItem` is memoized, so a change to the list renders only the rows it changes: an unmemoized row made every change to the list, a
loading line's included, render every row through its Markdown again, synchronously, for
half a second at a time. The browser's own scroll
anchoring is off on the list. A picture whose size is known keeps exactly its room before it
loads (`keptRoom` in `Attachments.tsx`), and one whose size is not keeps a square until it
arrives. The list's own scrolls never pin or unpin it from the bottom; a script's, as
find-in-page's, do, as the reader's do, judged against the content's height measured afresh,
since the browser also scrolls the box when what lies beneath it shrinks and the bottom it was
at moves up (the message box losing its files as a message is sent). `e2e/historyScroll.spec.ts` drags a phone back
through 600 messages of tall pictures and paragraphs, a few pixels at a time, resting the
finger before each lift, and fails if any step moves what is in view by other than the
finger's distance, or if the top of what is loaded, where a page would be awaited, ever comes
into view; a second test flicks back through the same history in quick, gathering flicks, and
allows the awaited top to show in at most `SEAM_SHARE` of its frames; a third reads back
through a history of short lines a little longer than the window and fails if any message in
view leaves the page, or if anything newer is read meanwhile. Tests scroll the list as a script does, by its position, which both ways of scrolling take.
The iOS simulator test `testReadingBackQuicklyNeverJumps` reads at a person's pace, with
flicks and drags that begin as soon as the last ended, against a build made with
`VITE_SCROLL_DEBUG=1`, in which the list records what it does with its position and a
watcher samples a row in view as every frame is about to be painted (in a resize observer
delivered after the list's own, so a change between frames that the list keeps still in the
next is not counted), counting as a jump any frame in which it moved by other than what the
list meant (`scrollDiagnostics.ts`, shown over the
list by `ScrollDiagnosticsPanel` with a copy of the record); it is how the pan's override
was found.

The code is in parts, each with one concern. `MessageList.tsx` renders the rows and joins the
rest. `ListScroller` (`listScroller.ts`) is a plain class, outside React's renders, that holds
the position and every step that sets it: noting and holding the still row, the pin to the
bottom, going to a linked message, the observers of the rows and the viewport, and on iOS the
fingers, wheel, keys, coasting, spring, and indicator. Its two hooks give it what each render
knows before it places the view (`useListPosition`), so its handlers, observers, and running
flings always act on the latest render's window and callbacks. `useHistoryPaging`
(`historyPaging.ts`) decides when a page is read, and is asked whenever the view moves or is
placed afresh; `useShownWindow` holds a change to the window while a deleted message's space
closes and shows it as a transition; `useNewMessagesLine` places the "New Messages" line;
`useReadMarking` marks what is seen as read.

## Moving between messages by keyboard

- The list is one stop in the tab order (`messageRows.ts`). Each row, a message
  (`MessageItem`), a notice, or a closed run of blocked messages (`BlockedRun`), carries
  `data-message-row` and takes `useRowProps`; one of them has `tabIndex` 0: the row last focused
  while it is still drawn, otherwise the newest. `MessageRows.settle` decides that after each
  render of the list, and a row asks with `useRowStop`, so moving re-renders only the rows the
  stop leaves and reaches. On a row itself (not a control inside it), Up and Down go to the row
  before and after and Home and End to the first and last the list holds; Tab goes on into the
  row's controls, which focus reveals as hover does. A move tells `ListScroller.focusMoved`
  which way it went, so history pages ahead of it as for a scroll, and the scroller's own keys
  leave alone a key a row took.

## Reading out arrivals

- Where the reader asks (`ANNOUNCE_MESSAGES`, kept with the device, under Settings,
  Accessibility), `useAnnounceArrivals` reads out each message that arrives in a list on screen
  (`RecordStore.arrivedAt` within the last ten seconds), as its author's name and its text
  (`describe`, as notifications word it), through `announce` (`src/features/layout/announce.ts`),
  which adds a node to the one polite live region `Announcer` keeps at the root. The reader's own
  messages, those of people they blocked, and history read in are left unsaid.
