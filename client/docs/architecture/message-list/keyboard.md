# Moving between messages by keyboard

The list is one stop in the tab order (`messageRows.ts`).

## Rows

- Each row carries `data-message-row` and takes `useRowProps`. A row is a message (`MessageItem`), a
  notice, or a closed run of blocked messages (`BlockedRun`).
- One row has `tabIndex` 0: the row last focused while it is still drawn, otherwise the newest.
- `MessageRows.settle` decides that after each render of the list.
- A row asks with `useRowStop`, so moving re-renders only the rows the stop leaves and reaches.

## Keys

On a row itself (not a control inside it):

| Key | Goes to |
| --- | --- |
| Up, Down | The row before, the row after. |
| Home, End | The first, the last row the list holds. |
| Tab | On into the row's controls, which focus reveals as hover does. |

## Paging and the scroller

- A move tells `ListScroller.focusMoved` which way it went, so history pages ahead of it as for a
  scroll.
- The scroller's own keys leave alone a key a row took.
