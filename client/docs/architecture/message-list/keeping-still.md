# Keeping the view still

What is in view never moves when something changes around it.

## The still row

The list notes a row and where it stands in the content (`still`). Scrolling does not change that
note.

Which row:

- normally, the topmost in view;
- while a linked message is shown, that message, held by its middle as long as that keeps its
  beginning in view (`linkedTop`). A jump then lands on it, centred, or with its beginning at the
  top of the view when it is taller than the view, and it stays so whatever loads around it or
  inside it: a picture of its own arriving moves its top up by half of the growth, no further than
  the top of the view.

The reader's first scroll drops the link, and the hold returns to the topmost row.

## After a change

After every change, the list moves the position by what the still row has moved. Changes include:

- a page's arrival, at commit;
- a picture's arrival, told by the picture itself in the same task (`useKeepStill`);
- any change of the rows' size seen by a `ResizeObserver` before the frame is painted.

## Catching up before the list's own moves

Every move of the list's own (a finger's or a fling's frame) first takes in whatever moved the still
row in the content since it was noted (`catchUp`). **Why:** those moves run before the frame's
resize observers are told, and would otherwise note the row where the change put it.

## Pictures' room

- A picture whose size is known keeps exactly its room before it loads (`keptRoom` in
  `Attachments.tsx`).
- One whose size is not known keeps a square until it arrives.

## Pinning to the bottom

- The list's own scrolls never pin or unpin it from the bottom.
- A script's scrolls (such as find-in-page's) do, as the reader's do.
- They are judged against the content's height measured afresh. **Why:** the browser also scrolls
  the box when what lies beneath it shrinks, and the bottom it was at moves up (the message box
  losing its files as a message is sent).
- A scroll that comes with a viewport height the list has not yet followed is the browser keeping
  the box's position within a viewport that grew, and does neither; the viewport's own handling
  decides (below). **Why:** a phone's keyboard going away grows the viewport in two steps (the room
  for the home indicator comes back a layout after the keyboard's goes), and the browser's position
  from the first lands short of the bottom, which taken as the reader's would let go of it.

## When the viewport changes size

A keyboard, the emoji keyboard, or a growing draft takes the viewport's height, and gives it back
(`observeViewport`).

| The list is          | Losing height                                                       | Gaining height                                                    |
| -------------------- | ------------------------------------------------------------------- | ----------------------------------------------------------------- |
| Pinned to the bottom | Stays at the bottom                                                 | Stays at the bottom                                               |
| Anywhere else        | Keeps its bottom edge, so what was just above the box stays in view | Keeps what is in view where it is; the room shows what lies below |

- Resizes that follow one another within `RESIZE_BURST_MS` count as one, judged by the height lost
  or gained across them all. **Why:** a keyboard's come in steps, and judging each alone would keep
  the bottom edge through one and the view through the next.
- **Why not keep the bottom edge on gaining too:** what was in view would slide down by the
  keyboard's height, showing history the reader had not scrolled to.
- How the emoji keyboard and a phone's take each other's place without the viewport changing at all
  is in [The message box](../composer-and-drafts.md).

## Scroll anchoring

The browser's own scroll anchoring is off on the list.
