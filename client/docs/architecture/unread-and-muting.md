# Unread and muting

## Where it lives

| Part | Where |
| --- | --- |
| Read states | `RecordStore` (topic `read:<channelId>`), `useReadState`, `useUnread` |
| Unread places | `unreadPlaces()` (topic `unread`), `useUnreadPlaces`, `UNREAD_DMS` |
| Marking read | `MessageList`, `AspenSync.markRead` |
| Thread positions | `threadRead` (topic `read:<threadId>`) |
| Mutes | `replaceMutes`, `useMute`, `MuteBell`, `useMuteEnd`, `nextMuteEnd`, `expireMutes` |
| The channel menu | `ChannelMenu`, `ChannelMenuButton` (`src/features/channels`) |

## Unread

### Read states

Unread is `RecordStore` state, one `ReadState` per channel. It comes from the `readStates`
sideload of the community list and the DM list.

A channel is unread while `lastMessage` sorts after `lastRead`.

| Event | Effect |
| --- | --- |
| Someone else's new message | Moves `lastMessage` |
| The caller's own new message | Moves `lastRead` |
| `channelRead` | Brings another device's reading |
| A deleted message that was a channel's `lastMessage` | `AspenSync` reads that channel's state again |

### Unread places

`unreadPlaces()` (`useUnreadPlaces`) names the communities with something unread, and `UNREAD_DMS`
for the DMs. They drive the rail's dots, which sit half under the entry's icon.

### Drawing unread

- An unread channel's icon and name, or a whole unread DM row, are marked with
  `unreadMarkClass`: an accent outline over a faint accent fill, padded so nothing moves.
- Every unread row's accessible name says so.
- In do not disturb nothing is drawn unread and no tag is counted: `useUnread`, `useMentions`,
  `usePlaceMentions`, and `useUnreadPlaces` answer as if all were read, and the rail, which reads
  the stores itself, does the same. The read states are untouched, so it all shows again when it
  ends, and the "New Messages" line, which says where reading left off, still shows (see
  [Presence](presence.md#do-not-disturb)).

### Marking read

`MessageList` marks the newest message on screen read through `AspenSync.markRead`.

- It updates the store at once.
- It reports to the server at most every `READ_REPORT_MS` per channel.
- It reports only while the page is visible and focused.
- Leaving the channel or hiding the page flushes the report.

### Threads

A thread's list marks it read the same way. A thread's position is kept apart (`threadRead`,
topic `read:<threadId>`):

- It comes from a read that sideloads it, and from `channelRead`.
- The caller's own replies move it.
- It never counts toward what the lists show unread.
- The activity feed reads it.

### The "New Messages" line

- It is placed where the read position was when the channel opened, and only if the channel was
  unread then.
- It stays there while the channel is open.
- Posting removes it.

## Muting

### Store state

- Mutes come from the `mutes` sideload of the community and DM lists, replaced whole at each
  bootstrap by `replaceMutes`.
- `channelMuteChanged` events keep them current.
- `AspenSync` ends timed mutes by the device's clock (`nextMuteEnd`, `expireMutes`), since the
  server announces no end it did not make.

### A muted channel or DM

A muted channel or DM (`useMute`):

- is drawn in `text-ink-faint`;
- shows a muted bell (`MuteBell`): a focusable image whose tooltip says until when, in the words
  of the Mute submenu (`useMuteEnd`). In the DM list the bell sits beside the options button
  rather than in the row's link, which may hold nothing focusable;
- is never marked unread;
- does not count toward `unreadPlaces`.

Its read position is kept, so it is unread again once the mute ends. The "New Messages" line
still shows inside it.

### The channel menu

`ChannelMenu` (`src/features/channels`) holds two submenus above the channel's other actions:

| Submenu | Offers |
| --- | --- |
| Mute | One of the offered lengths; or, while muted, until when and Unmute |
| Notifications | Names the level in force |

It opens:

- on a right click on the row of a text channel, a DM, or a channel of a plugin's kind (whose
  plugin's notices its mute and level govern);
- from the row's `ChannelMenuButton`, which keyboards and touch screens use, since a long press on
  a row starts dragging it.

It opens beside the row, and below it on a one-pane screen, where there is no room beside it.
