# What a row holds

A window is up to `LIVE_WINDOW_MAX_MESSAGES` rows, all of them drawn, so whatever one row keeps is
kept hundreds of times over. Most of what a row could keep is for controls nobody is using.

## What costs

- A React Aria `Button` holds close to a hundred hooks (press, hover, focus ring, and their
  listeners); a `ToggleButton` about eighty; a `TooltipTrigger` about fifty, and its closed
  `Tooltip` thirty more; a closed `Popover` or `ModalOverlay` over a hundred.
- A hook is about a quarter of a kilobyte of JavaScript heap, so a row that built its ten actions,
  their tooltips, and their dialogs outright would hold about half a megabyte, and a window of 300
  such rows 150 MB.

## Actions only on the rows in use

`MessageItem` builds `MessageActions` only while its row is engaged (`useRowEngagement`,
`messageRows.ts`).

- **Two rows of a list are engaged:** the row the pointer last entered or moved over, and the row
  focus last went into. A list's `MessageRows` holds both, as it holds the tab stop.
- **A row stays engaged until another takes its place.** What its actions opened (a dialog, the
  emoji picker) outlives the pointer leaving the row, and has its button to give focus back to when
  it closes. Those are modal, so neither place can be taken while one is open.
- **A row not engaged keeps one button where its actions go** (`ActionsStandIn`), named "Show
  message actions". Focus reaching it engages the row, and focus goes on to the first of the
  actions, or the last when it came from after them, in the same task; so Tab and Shift+Tab meet
  the actions in the order they always had. Assistive technology reading through a channel finds
  that one button in each message rather than every action of every message; activating it brings
  the actions.
- **The stand-in is a plain `button`.** It is never seen or pressed by a pointer, and one stands in
  every row, so it holds no state.
- A touch screen builds a message's sheet from its first long press (`pressed`), as before.

## Overlays only once opened

`OnceOpen` (`src/features/layout/OnceOpen.tsx`) builds what it wraps from when it first opens: the
state of the trigger it stands in (`DialogTrigger`, `MenuTrigger`), or `isOpen` for an overlay its
owner opens. It stays built afterwards, so it can move as it closes.

- `Tooltip` wraps its React Aria tooltip in `TooltipOnceOpen`, so every tooltip in the app is built
  the first time it shows.
- In a row: the profile cards of the author's picture and name (`ProfilePopover`), the reactions
  dialog and the reaction picker, the picture gallery, a poll's voters and write-in dialogs, a
  plugin's annotation, and a link's menu.
- Beside the list: each channel's and each DM's menu (`ChannelMenu`).

Wrap an overlay in `OnceOpen` wherever a list gives each of its rows one.

## Measuring

Counting fibers and hooks under `article[data-message-id]` (each fiber's `memoizedState` chain),
by component, says where a row's weight is; a heap snapshot says what it comes to. A row of plain
text is about 14 elements and 200 hooks, and a window of 300 of them about 40 MB of heap.
