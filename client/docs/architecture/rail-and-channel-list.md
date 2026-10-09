# The community rail and the channel list

## Where it lives

| Part | Where |
| --- | --- |
| Drop to new order | `src/features/layout/reorder.ts` |
| Reordering requests | `AspenSync.reorderCommunities`, `AspenSync.reorderChannels`, `arrangeChannels` |
| The rail and folders | `CommunityRail`, `RailFolder.tsx` |
| What a rail drop means | `moveInRail` (`railOrder.ts`) |
| Folded categories | `ChannelSidebar`, `useCollapsed`, `AspenSync.setCategoryCollapsed` |

## Reordering

Reordering is drag and drop from React Aria: `useDragAndDrop` on a `GridList`.

- There is one `GridList` for the community rail and one per channel group.
- Each row carries a `Button slot="drag"` handle, which keyboard and screen reader users drag
  with.
- `src/features/layout/reorder.ts` turns a drop into the new id order.
- Rows carry a private drag type, so channel groups accept only channels and the rail accepts
  only communities.

`AspenSync.reorderCommunities` and `reorderChannels`:

1. renumber positions;
2. patch only the records whose index changed;
3. arrange the cache at once, so the events that follow are no-ops.

| What | Order kept in |
| --- | --- |
| Communities | Per user, on the caller's membership (`UserCommunity.sortIndex`), which every community read sideloads |
| Channels | Position, and `parentCategory` when moved |
| Categories | Their own order, for now |

A channel dragged into another group, or onto an empty category's drop zone, moves there.
`arrangeChannels` sets its `parentCategory` along with its position.

## Folders on the rail

Communities gather into folders on the rail, as apps do on an iPhone's home screen
(`CommunityRail`, `RailFolder.tsx`).

### Storage

| Preference | Key | Holds |
| --- | --- | --- |
| `RAIL_FOLDERS` | `rail.folders` | Each folder's id, name, tint from `FOLDER_COLORS`, whether it is open, and its members' rail keys in order |
| `RAIL_ORDER` | | The rail order; names a folder where it stands as `folder:{id}` |

- Both are account preferences, written together in one request (`PreferenceStore.setAccount`).
- A client that knows nothing of folders shows their communities at the end of its rail.

### One list

The rail stays one `GridList`. An open folder's communities follow its row, on its tint. Pointer,
touch, and keyboard dragging and screen readers work the same throughout.

### What a drop means

`moveInRail` decides; its tests in `railOrder.ts` list every case.

| Dropped | Onto or where | Result |
| --- | --- | --- |
| A community | Another community | Makes a folder of the two |
| A community | A folder, or one of its communities | Joins it |
| A community | Among an open folder's communities | Goes in there |
| A community | After an open folder's last community | Comes out |
| A folder | Anywhere | Moves whole, never onto anything |

A folder left with one community goes.

### Closed and open folders

- A closed folder shows its first four icons two by two.
- It speaks for its communities: the unread mark, the tag count, the current community's ring.
- Pressing it opens it in place.
- Its menu, from a right click or its options button, renames, tints, and ungroups it.
- Each deployment still gets its own share of the order, folders opened in place
  (`railSequence`).

## Folded categories

Folded categories are store state.

- They come from the `collapses` sideload of the community list, replaced whole at each
  bootstrap by `replaceCollapsed`.
- `categoryCollapseChanged` events keep them current (`useCollapsed`).
- A category's heading in `ChannelSidebar` is a button that folds it
  (`AspenSync.setCategoryCollapsed`, `aria-expanded`).

A folded category still lists:

- the channel being viewed;
- whatever `RecordStore.shownWhenCollapsed` keeps: an unread, unmuted channel, or a voice channel
  with someone in the call. `useShownWhenCollapsed` follows them.

Drops and reorders in a folded category still place channels among all of its channels, shown or
not.
