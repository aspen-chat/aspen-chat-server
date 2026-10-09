# Message groups

A run of one author's messages is drawn as a group (`messageGroups.ts`, worked out by
`useGroupContinuations` in `MessageList`).

- The first message shows the author's picture, name, and time.
- The rest sit under it with none of them.
- Each is still its own row, with a little padding between, so it highlights, takes focus, and opens
  its actions alone.

## When a message continues a group

A message continues the group before it when all of these hold:

- It is by the same author.
- It is on the same day, where the reader is.
- It is within 30 minutes of the group's first message (`GROUP_SPAN_MS`).
- Nothing is between them (the New Messages line, a run of blocked messages).
- Neither is a kind that stands alone. Only `standard` and `poll` group. Notices, calls, closed polls,
  thread echoes, commands, and warnings stand alone.
- The group's height stays within three quarters of the list's (`GROUP_HEIGHT_SHARE`).

## Estimated height

The group's height is estimated, never measured (`estimateHeight`):

- text wrapped at the column's width, from the list's font size, with wide characters counted twice;
- pictures at their recorded sizes, as they are drawn;
- fixed heights for files, link cards, polls, reaction chips, and thread summaries.

So a group is the same however its pictures load.

## When groups are decided

- Groups are worked out oldest first over the window whenever it changes.
- A message keeps what it was decided to be (begins a group, or continues one) for as long as it
  stays in the window and the message before it is still its author's with nothing between.
  **Why:** read afresh, a page of older history arriving above the view would move group boundaries
  all the way down into it.
- Resizing the list decides them all again.

## Time and assistive technology

- A grouped message keeps its author and time for assistive technology (visually hidden).
- On a computer its time shows after whatever it ends with (`MessageBody`'s `trailing`) while the
  pointer is over it or focus is in it.
- On a touch screen its actions' sheet says when it was sent, as every message's does.

### Where the time sits

- After text, it follows the edited mark, level with the text's top: on its first line's baseline,
  or at the top edge of a code block or table the text opens with (`.message-text-row`).
- Beside a picture, poll, or card (`Trailed`), it sits level with the block's top where the line
  has room, and under the block where it has not.
- Where it would reach under the hover actions, `MessageItem` measures them on hover and starts
  it just below their bottom edge (`trailingDrop`).
