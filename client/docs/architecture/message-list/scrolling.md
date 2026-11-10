# Who scrolls the list

| Platform        | Who scrolls                                     | Switch                                |
| --------------- | ----------------------------------------------- | ------------------------------------- |
| iOS and iPadOS  | The list scrolls itself.                        | `OWNS_SCROLLING` in `listScroller.ts` |
| Everywhere else | The browser scrolls the box, on its compositor. |                                       |

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

### Fingers

What a finger does is what it does on iOS's own lists, which `UIScrollView` decides there and the
list decides here.

- **A tap moves nothing.** Until the finger has gone `DRAG_SLOP_PX` from where it touched, the list
  stays still, so a tap with a little shake in it leaves the list, and the URL's linked message,
  where they were.
- **Past the slop it is dragging**, and the list follows it from there: the move that crosses the
  slop moves the list by what it went past the slop, so the content never jumps by the slop as the
  drag begins.
- **A drag is not a press.** Whatever the finger touched is told so, as the browser tells it when a
  pan of its own takes the touch: the rows take no pointer until the finger lifts, and the touched
  element is sent a `pointercancel`, which ends the press the browser's `pointerdown` began before
  the list's `touchstart` arrived (React Aria's, and so a message's long press; a self link's and a
  reaction's own). The lift is no click either (`preventDefault` on `touchend`), so lifting over a
  link, a picture, or a button opens nothing.
- **A drag puts the keyboard away** (`dismissKeyboard`, `src/features/layout`): the field holding
  the focus lets it go, and the emoji keyboard standing in the phone's keyboard's place goes with it
  (see [the composer](../composer-and-drafts.md#the-emoji-keyboard)).
- **A touch that catches the list moving stops it, and is nothing else.** While the list coasts
  after a fling or springs back from past an end, a touch stops it where it is and is treated as a
  drag from its first moment: no press, no long press, and no click on whatever came to rest under
  it. **Why:** readers stop a fling with a finger wherever it lands, and a link there would open
  under it; iOS's own lists treat the stopping touch the same way, so hands expect it. The finger
  may go on to drag from there, past the slop as any other.

### The indicator

- It is drawn over the view, in the box around the viewport, never in the viewport: an absolute
  child of a scroll container is laid out against the content at scroll position zero and scrolls
  away with it, so a track inside the viewport would be seen at the top of the history alone. The
  Jump to latest pill, which is inside the viewport, stays over the view by being sticky instead.
- It is drawn only while there is more than fits, and shows for `INDICATOR_MS` after each move of
  the list (`moving` on the track; its opacity is the component's styles', never set inline).
- It flashes once as the list appears, the first time it is placed since it was reset for a channel,
  as iOS's own lists flash theirs, telling the reader there is more than fits.

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
