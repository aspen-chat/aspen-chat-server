# Scrolling tests

Tests scroll the list as a script does, by its position, which both ways of scrolling take.

## Playwright: `e2e/historyScroll.spec.ts`

The Chromium history tests run both ways of scrolling. Chromium stands in for iOS by claiming the
property the list knows it by.

| Test | What it does | Fails if |
| --- | --- | --- |
| Dragging back | Drags a phone back through 600 messages of tall pictures and paragraphs, a few pixels at a time, resting the finger before each lift. | Any step moves what is in view by other than the finger's distance, or the top of what is loaded (where a page would be awaited) ever comes into view. |
| Flicking back | Flicks back through the same history in quick, gathering flicks. | The awaited top shows in more than `SEAM_SHARE` of its frames. |
| Reading back | Reads back through a history of short lines a little longer than the window. | Any message in view leaves the page, or anything newer is read meanwhile. |

## iOS simulator: `testReadingBackQuicklyNeverJumps`

1. It runs against a build made with `VITE_SCROLL_DEBUG=1`. In that build the list records what it
   does with its position.
2. It reads at a person's pace, with flicks and drags that begin as soon as the last ended.
3. A watcher samples a row in view as every frame is about to be painted. It runs in a resize
   observer delivered after the list's own, so a change between frames that the list keeps still in
   the next is not counted.
4. A jump is any frame in which the row moved by other than what the list meant.

The record is in `scrollDiagnostics.ts`, shown over the list by `ScrollDiagnosticsPanel` with a copy
of the record. This test is how the pan's override was found (see
[design notes](design-notes.md#why-the-list-scrolls-itself-on-ios)).
