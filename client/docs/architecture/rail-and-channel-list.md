# The community rail and the channel list

- Reordering is drag and drop from React Aria (`useDragAndDrop` on a `GridList` for the
  community rail and one per channel group, each row carrying a `Button slot="drag"` handle
  that keyboard and screen reader users drag with), with `src/features/layout/reorder.ts`
  turning a drop into the new id order. `AspenSync.reorderCommunities` and `reorderChannels`
  renumber positions, patch only the records whose index changed, and arrange the cache at
  once so the events that follow are no-ops. Community order is per user: it lives on the
  caller's membership (`UserCommunity.sortIndex`), which every community read sideloads.
  A channel dragged into another group, or onto an empty category's drop zone, moves there:
  `arrangeChannels` sets its `parentCategory` along with its position. Rows carry a private
  drag type so channel groups accept only channels and the rail accepts only communities.
  Categories keep their own order for now.
- Communities gather into folders on the rail, as apps do on an iPhone's home screen
  (`CommunityRail`, `RailFolder.tsx`). Folders are the account preference `RAIL_FOLDERS`
  (`rail.folders`: each folder's id, name, tint from `FOLDER_COLORS`, whether it is open, and
  its members' rail keys in order), and `RAIL_ORDER` names a folder where it stands as
  `folder:{id}`; the two are written together in one request (`PreferenceStore.setAccount`),
  and a client that knows nothing of folders shows their communities at the end of its rail.
  The rail stays one `GridList`, an open folder's communities following its row on its tint,
  so pointer, touch, and keyboard dragging and screen readers work the same throughout. What
  a drop means is `moveInRail` (`railOrder.ts`, whose tests list every case): a community
  dropped on another makes a folder of the two, on a folder or one of its communities joins
  it, among an open folder's communities goes in there, and after its last comes out; a folder
  moves whole and never onto anything; a folder left with one community goes. A closed folder
  shows its first four icons two by two and speaks for its communities (unread mark, tag
  count, the current community's ring); pressing it opens it in place. Its menu, from a right
  click or its options button, renames, tints, and ungroups it. Each deployment still gets its
  own share of the order, folders opened in place (`railSequence`).
- Folded categories are store state from the `collapses` sideload of the community list
  (replaced whole at each bootstrap by `replaceCollapsed`), kept current by
  `categoryCollapseChanged` events (`useCollapsed`). A category's heading in `ChannelSidebar`
  is a button that folds it (`AspenSync.setCategoryCollapsed`, `aria-expanded`); folded, it
  still lists the channel being viewed and whatever `RecordStore.shownWhenCollapsed` keeps (an
  unread, unmuted channel, or a voice channel with someone in the call; `useShownWhenCollapsed`
  follows them). Drops and reorders in a folded category still place channels among all of
  its channels, shown or not.
