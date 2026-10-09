# Who scrolls the list

| Platform | Who scrolls | Switch |
| --- | --- | --- |
| iOS and iPadOS | The list scrolls itself. | `OWNS_SCROLLING` in `listScroller.ts` |
| Everywhere else | The browser scrolls the box, on its compositor. | |

On every platform:

- The browser's scroll anchoring is off on the list, since the list keeps its own view still (see
  [keeping the view still](keeping-still.md)).
- What the list does with the position is the same either way.
- Nothing outside the list may assume either way.

## Where the list scrolls itself (iOS)

`ListScroller` holds the list's position and everything that sets it, with the arithmetic in
`scrollPhysics.ts`.

- The list's box hides its overflow, so no finger, wheel, or key scrolls it.
- The list takes those itself and sets the box's scroll position from them.
- It draws its own coasting, spring at the ends, and indicator.
- Scripts still scroll the box. Focus, find-in-page, assistive technology, and Playwright bring
  things into view as they always did.

**Why:** iOS overrides any position set during a pan of a box the browser scrolls (see
[design notes](design-notes.md#why-the-list-scrolls-itself-on-ios)).

### Dragging

- Once a finger has moved `DRAG_SLOP_PX`, it is dragging.
- The rows then take no pointer until it lifts. Lifting it over a picture or a button is not a press,
  as it would not be under a pan the browser made.

### Which events it takes

The list takes only what happens in the list itself. React passes events up through what a row opens
in a layer of its own over the page (the actions' sheet, the emoji picker's, a dialog) as though it
were in the row. Without this check, a finger scrolling a picker there would drag the list behind
it.

## Where the browser scrolls

Everywhere else the browser scrolls the box, on its compositor, with its own indicator, overscroll,
and assistive gestures. It honours a position the list sets at any moment.

## Giving scrolling back to the browser on iOS

The conditions for giving scrolling back to the browser on iOS too, and the experiment that proves
them, are in `MessageList.tsx`'s header.
