# Unread and muting

- Unread is `RecordStore` state from the `readStates` sideload of the community list and the
  DM list, one `ReadState` per channel (topic `read:<channelId>`; `useReadState`, `useUnread`),
  kept current by events: someone else's new message moves `lastMessage`, the caller's own
  moves `lastRead`, `channelRead` brings another device's reading, and a deleted message that
  was a channel's `lastMessage` makes `AspenSync` read that channel's state again. A channel is
  unread while `lastMessage` sorts after `lastRead`. `unreadPlaces()` (topic `unread`,
  `useUnreadPlaces`) names the communities with something unread, and `UNREAD_DMS` for the DMs,
  for the rail's dots, which sit half under the entry's icon. An unread channel's icon and name,
  or a whole unread DM row, are marked with `unreadMarkClass` (an accent outline over a faint
  accent fill, padded so nothing moves), and every unread row's accessible name says so. `MessageList` marks the newest message on screen read through
  `AspenSync.markRead`, which updates the store at once and reports to the server at most every
  `READ_REPORT_MS` per channel, and only while the page is visible and focused; leaving the
  channel or hiding the page flushes the report. The "New Messages" line is placed where the
  read position was when the channel opened, and only if it was unread then, and stays there
  while the channel is open; posting removes it.
- Muting is store state from the `mutes` sideload of the community and DM lists (replaced
  whole at each bootstrap by `replaceMutes`), kept current by `channelMuteChanged` events;
  `AspenSync` ends timed mutes by the device's clock (`nextMuteEnd`, `expireMutes`), since the
  server announces no end it did not make. A muted channel or DM (`useMute`) is drawn in
  `text-ink-faint` with a muted bell, is never marked unread, and does not count toward
  `unreadPlaces`; its read position is kept, so it is unread again once the mute ends, and the
  "New Messages" line still shows inside it. `ChannelMenu` (`src/features/channels`) holds two
  submenus, Mute (one of the offered lengths, or, while muted, until when and Unmute) and
  Notifications (naming the level in force), above the channel's other actions; it opens on a
  right click on a text channel's or DM's row, or from the row's `ChannelMenuButton`, which
  keyboards and touch screens use, since a long press on a row starts dragging it. It opens
  beside the row, and below it on a one-pane screen, where there is no room beside it.
