# Unread and muting

## Where it lives

| Part                            | Where                                                                                                              |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Read states                     | `RecordStore` (topic `read:<channelId>`), `useReadState`, `useUnread`                                              |
| Unread places                   | `unreadPlaces()` (topic `unread`), `useUnreadPlaces`, `UNREAD_DMS`                                                 |
| Unread channels                 | `unreadChannels()` (topic `unread`), which the activity feed's Unread tab lists                                    |
| Marking read                    | `MessageList`, `AspenSync.markRead`                                                                                |
| Marking unread                  | `AspenSync.markUnread`, `markUnread`, `MarkUnreadButton`, `ChannelMenu`, `unreadHeld`                              |
| Thread positions                | `threadRead` (topic `read:<threadId>`)                                                                             |
| Mutes                           | `replaceMutes`, `useMute`, `useOwnMute`, `useCategoryMute`, `MuteBell`, `useMuteEnd`, `nextMuteEnd`, `expireMutes` |
| Mute and Notifications submenus | `ChannelMuteSubmenu`, `CategoryMuteSubmenu`, `NotificationsSubmenu` (`src/features/channels/muteMenus.tsx`)        |
| The channel menu                | `ChannelMenu`, `ChannelMenuButton` (`src/features/channels`)                                                       |

## Unread

### Read states

Unread is `RecordStore` state, one `ReadState` per channel. It comes from the `readStates` sideload
of the community list and the DM list.

A channel is unread while `lastMessage` sorts after `lastRead`.

| Event                                                | Effect                                                |
| ---------------------------------------------------- | ----------------------------------------------------- |
| Someone else's new message                           | Moves `lastMessage`                                   |
| The caller's own new message                         | Moves `lastRead`                                      |
| `channelRead`                                        | Brings another device's reading                       |
| `channelMarkedUnread`                                | Moves the position back, and holds the channel unread |
| A deleted message that was a channel's `lastMessage` | `AspenSync` reads that channel's state again          |

### Unread places

`unreadPlaces()` (`useUnreadPlaces`) names the communities with something unread, and `UNREAD_DMS`
for the DMs. They drive the rail's dots, which sit half under the entry's icon.

### Drawing unread

- An unread channel's icon and name, or a whole unread DM row, are marked with `unreadMarkClass`: an
  accent outline over a faint accent fill, padded so nothing moves.
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

The activity feed marks places read without opening them, one at a time through `markRead` and all
at once through `AspenSync.markManyRead`, which reports many positions in one request (see
[Saved messages and activity](saved-and-activity.md)).

### Marking unread

The one way a position moves back. It is offered:

- in a message's actions, hover bar and long-press sheet alike (`MarkUnreadButton`), on messages by
  someone else, where the caller keeps a position: a channel or DM they belong to, or a thread;
- in `ChannelMenu`, on a channel or DM read to its end, from its newest message by someone else,
  which only the server finds.

`AspenSync.markUnread`:

1. drops that channel's read not yet reported, which would undo the marking;
2. moves the store back at once when a message is named (`positionBefore`, the id less one) and
   holds the channel unread (`unreadHeld`);
3. sends the request, after which `channelMarkedUnread` brings the state the server counted.

A refusal releases the hold, reads the state back, and is said in a toast, as success is.

#### The hold

While a channel is held unread, `useReadMarking` marks nothing in it read as it is shown, so it
stays unread on the screen it was marked on. Marking on another device holds it on every device. The
hold ends when:

- the channel's list is opened or left (`useReadReports`);
- the caller posts there;
- the caller marks it read by hand (`markRead`, `markManyRead`).

The "New Messages" line follows the position while the channel is held.

### Threads

A thread's list marks it read the same way. A thread's position is kept apart (`threadRead`, topic
`read:<threadId>`):

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

- Mutes come from the `mutes` sideload of the community and DM lists: each channel's own setting
  (`channelMutes`, a mute or kept unmuted) and each category's mute (`categoryMutes`), replaced
  whole at each bootstrap by `replaceMutes`.
- `channelMuteChanged` and `categoryMuteChanged` events keep them current.
- `AspenSync` ends timed mutes by the device's clock (`nextMuteEnd`, `expireMutes`), since the
  server announces no end it did not make.
- `RecordStore.mute` says the mute in force on a channel (topic `mute:<channelId>`), as the server
  decides it (`docs/architecture/muting.md`): its own setting in force, else its category's mute,
  naming that category. A category's mute, and a channel arriving in or moving to another category,
  touch the topic of each channel it holds. `ownMute` is the channel's own setting, which the Mute
  submenu reads.

### A muted channel or DM

A muted channel or DM (`useMute`), by its own mute or its category's:

- is drawn in `text-ink-faint`;
- shows a muted bell (`MuteBell`): a focusable image whose tooltip says until when, in the words of
  the Mute submenu (`useMuteEnd`), and that it is the category's when it is. A muted category's
  heading shows one too. In the DM list the bell sits beside the options button rather than in the
  row's link, which may hold nothing focusable;
- is never marked unread;
- does not count toward `unreadPlaces`.

Its read position is kept, so it is unread again once the mute ends. The "New Messages" line still
shows inside it.

### The channel menu

`ChannelMenu` (`src/features/channels`) holds two submenus above the channel's other actions:

| Submenu       | Offers                                                                     |
| ------------- | -------------------------------------------------------------------------- |
| Mute          | See below; it reads until when in its own place while the channel is muted |
| Notifications | Names the level in force                                                   |

Mute and Notifications are offered on a text channel, a DM, and a channel of a plugin's kind (whose
plugin's notices its mute and level govern). The menu holds more besides; see
[context-menus.md](context-menus.md).

The Mute submenu (`ChannelMuteSubmenu`) offers, by the channel's state:

| State                                | Offers                                                                                                                                           |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Not muted                            | The lengths of mute                                                                                                                              |
| Muted by its own mute                | Unmute: removing its own mute, or, while its category is muted too, keeping it unmuted, since removing it would leave it muted with the category |
| Muted with its category              | Keep unmuted                                                                                                                                     |
| Kept unmuted inside a muted category | The lengths of mute, and Follow its category, which removes its own setting                                                                      |

A category's heading menu has a Mute submenu of its own (`CategoryMuteSubmenu`): the lengths, or
until when and Unmute.

It opens:

- on a right click on the channel's or DM's row, at the pointer;
- from the row's `ChannelMenuButton`, which keyboards and touch screens use, since a long press on a
  row starts dragging it. It opens beside the button, and below it on a one-pane screen, where there
  is no room beside it.
