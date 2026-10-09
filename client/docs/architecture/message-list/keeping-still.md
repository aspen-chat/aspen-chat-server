# Keeping the view still

What is in view never moves when something changes around it.

## The still row

The list notes a row and where it stands in the content (`still`). Scrolling does not change that
note.

Which row:

- normally, the topmost in view;
- while a linked message is shown, that message, held by its middle. A jump then lands on it and it
  stays centred whatever loads around it or inside it.

The reader's first scroll drops the link, and the hold returns to the topmost row.

## After a change

After every change, the list moves the position by what the still row has moved. Changes include:

- a page's arrival, at commit;
- a picture's arrival, told by the picture itself in the same task (`useKeepStill`);
- any change of the rows' size seen by a `ResizeObserver` before the frame is painted.

## Catching up before the list's own moves

Every move of the list's own (a finger's or a fling's frame) first takes in whatever moved the still
row in the content since it was noted (`catchUp`). **Why:** those moves run before the frame's resize
observers are told, and would otherwise note the row where the change put it.

## Pictures' room

- A picture whose size is known keeps exactly its room before it loads (`keptRoom` in
  `Attachments.tsx`).
- One whose size is not known keeps a square until it arrives.

## Pinning to the bottom

- The list's own scrolls never pin or unpin it from the bottom.
- A script's scrolls (such as find-in-page's) do, as the reader's do.
- They are judged against the content's height measured afresh. **Why:** the browser also scrolls the
  box when what lies beneath it shrinks, and the bottom it was at moves up (the message box losing
  its files as a message is sent).

## Scroll anchoring

The browser's own scroll anchoring is off on the list.
