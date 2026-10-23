# Saved messages and activity

Two pages of the reader's own, across every deployment they use (`useSources`).

## Where it lives

| Part                 | Where                                                                              |
| -------------------- | ---------------------------------------------------------------------------------- |
| Rail links           | `CommunityRail`, `railLinkClass`                                                   |
| Page layout          | `PersonalPage` (`src/features/activity`), `RailPage` (`src/features/layout`)       |
| A listed message     | `ListedMessage` (`src/features/messages`), `useMessagePlace`, `useMessageOnDemand` |
| Saved messages       | `SavedScreen`, `SaveButton`, `SavedMark`, `RecordStore.saves`                      |
| The activity feed    | `ActivityScreen`, `ActivityFilters`, `readFilter`                                  |
| Its tabs             | `ACTIVITY_TAB`, `FeedTab`, `UnreadTab`, `useFeeds` in `ActivityScreen`             |
| What is unread       | `unreadEntries` (`unread.ts`), `RecordStore.unreadChannels`, `ChannelItem`         |
| A thread in the feed | `foldThreads` (`threads.ts`), `ThreadItem` in `ActivityScreen`                     |

## Opening the pages

One round link at the top of the community rail (`CommunityRail`), above the DMs', opens Activity.
It is drawn as the DMs' is (`railLinkClass`) and ringed while either page is open. Saved messages
open from the page's own rail (`PersonalPage`), which goes between the two.

| Page           | Route       |
| -------------- | ----------- |
| Activity       | `/activity` |
| Saved messages | `/saved`    |

They are never under `/at/{domain}`, since they span deployments.

## Layout

Both are `PersonalPage`s (`src/features/activity`), a `RailPage` (`src/features/layout`).
`RailPage` is the layout the Administration Dashboard uses too.

- Its rail goes between the two pages and holds whatever more the page puts in it.
- The user bar is at the rail's foot.
- Their content scrolls only up and down, so a phone never pans them sideways.
- What is wider than the column (a code block) scrolls in itself.
- The filters' long names are cut short.

## Listed messages

Each message the pages list is a `ListedMessage` (`src/features/messages`, which a channel's pins
use too). It shows:

- its author;
- when;
- where it was said (`useMessagePlace`, which search's results use too): the governing channel,
  the thread, the link that goes to it, and a line naming the place, with `on {domain}` for
  another deployment;
- a way to go to it, and actions beside that;
- the message, drawn as its channel draws it.

Each row:

- runs in its deployment's `SourceScope`;
- reads its message on demand (`useMessageOnDemand`).

A row's text is the size of the list it stands in. Both pages' lists are `message-text`, so their
rows are drawn at the reader's message text size and line spacing (`preferences.md`), 16 pixels
at the normal, as a channel draws them. A channel's pins keep `text-sm`.

A message the server refuses or no longer has is marked missing in the store (`MissingKind`
`message`), and its row leaves the list. So a message deleted, or a channel lost, takes its rows
with it.

## Saved messages

### Store state

| State               | Topic               | Holds                        |
| ------------------- | ------------------- | ---------------------------- |
| `RecordStore.saves` | `saved`             | Saves, newest save first     |
| `isSaved`           | `saved:<messageId>` | Whether one message is saved |

- Read whole at bootstrap (`replaceSaves`) from `GET /users/@me/saved-messages`. A deployment
  without it answers with nothing.
- Kept current by `savedMessageChanged`.
- `AspenSync.setSaved` saves and unsaves, and the store follows at once.

### In messages

- Every message's actions offer Save message or Remove from saved (`SaveButton`), in the hover bar
  and the long-press sheet alike.
- A refusal (the deployment keeps as many as it will) is said in a toast.
- A saved message carries a quiet bookmark (`SavedMark` in `MessageBody`). It is named for
  assistive technology. Where it goes:

  | Screen    | Message                 | Bookmark                                                        |
  | --------- | ----------------------- | --------------------------------------------------------------- |
  | Not touch | Any                     | After whatever it ends with, kept with a grouped message's time |
  | Touch     | With a header           | After the time in the message's header                          |
  | Touch     | Grouped (shows no time) | After whatever it ends with (`MessageBody`'s `savedMark`)       |

### `SavedScreen`

1. It merges every deployment's saves by their ids. They are UUIDv7s, which compare by time
   across deployments.
2. It shows `SAVED_PAGE` at a time, with Show more.
3. It reads each deployment's messages a page at a time (`AspenSync.loadSavedMessages`), as far
   as its saves among those shown.

Each row removes its save.

## The activity feed

`ActivityScreen` shows what is said where the reader reads, under one of five tabs. The tab is a
device preference (`ACTIVITY_TAB`), Unread until one is chosen.

| Tab      | Shows                                                         | Read with                                                             |
| -------- | ------------------------------------------------------------- | --------------------------------------------------------------------- |
| Unread   | Every unread channel, DM, and followed thread, each one entry | The store's read states, and the `threads` view with `filter[unread]` |
| Mentions | Messages that tag the reader                                  | The `mentions` view                                                   |
| DMs      | The reader's DMs' messages                                    | The `dms` view                                                        |
| Threads  | Replies in the threads they follow                            | The `threads` view                                                    |
| All      | Everything said where they read                               | The `all` view                                                        |

Muted channels are in none of them.

### Reading a view

`FeedTab` lists one view's messages, read or not (`useFeeds`):

1. It reads each deployment's page (`AspenSync.readActivity`, `ACTIVITY_PAGE` at a time), with
   read positions sideloaded.
2. It merges them as search does (`mergeResults`).
3. It reads older pages on request.

What arrives while a view is open (`AspenSync.onArrival`) joins the top of its deployment's feed
if the view holds it (`RecordStore.inActivity`, the server's rule for each view) and the filter
shows it.

### Unread

`UnreadTab` lists places, not messages, so a busy channel is one entry:

- Channels and DMs come from each deployment's store (`RecordStore.unreadChannels`, topic
  `unread`), which holds every one whole and follows them live.
- Threads come from each deployment's `threads` view with `filter[unread]`, since no list carries
  threads' positions. A thread read since leaves the list (`threadRead`).
- `unreadEntries` orders both by their newest unread message, and holds back what is older than
  the oldest reply of a deployment with more replies to give.
- `UNREAD_PAGE` entries show at a time, since each reads its channel's latest page.

A channel's entry (`ChannelItem`) shows where it is, its `ENTRY_MESSAGES_SHOWN` (five) latest
unread messages, and how many are unread as far as its window reaches (that many "or more" when
the whole window is unread and older messages remain).

### Marking read

Showing the feed marks nothing read.

- Every unread entry and unread message has a Mark read button (`AspenSync.markRead`). On a
  message it moves the channel's position up to that message.
- Mark all read, on the Unread tab, marks every channel the filter shows and every thread read so
  far (`AspenSync.markManyRead`, `READ_REPORTS_MAX` positions to a request), then reads the
  threads again, since more may wait behind those read.

In the other tabs a message after the reader's position where it was said carries an unread dot.
The position is a channel's, or a thread's own (`useLastRead`).

### Threads

A thread's replies in the feed stand as one entry (`foldThreads`), where the newest of them
stands. Its older replies there are left out. The entry (`ThreadItem`) shows:

1. where the thread is, an unread dot when its newest reply in the feed is unread, and a way to
   the thread;
2. the thread's `ENTRY_MESSAGES_SHOWN` (five) latest replies, oldest first, whoever wrote them,
   the reader's own among them;
3. the thread's `Composer`, always, so the reader replies without leaving.

The replies are the thread's window in the store (`useMessageWindow`), so they follow the thread
live.

| The store's window            | What the entry does                                                   |
| ----------------------------- | --------------------------------------------------------------------- |
| Reaches the latest            | Shows its last five                                                   |
| Missing, or not at the latest | Reads the latest page (`AspenSync.loadLatest`), in skeleton meanwhile |
| The read failed               | Shows the feed's newest reply alone                                   |

An entry subscribes to its thread's window, so the store keeps one window for each thread the
feed shows, however many (`KEPT_WINDOWS_MAX` drops only windows nothing shows).

The entry is keyed by its thread, so what is being written in it stays when a newer reply moves
it up the feed. A thread deleted since, or left with no replies, leaves the feed.

### Filters

What every tab shows is narrowed in the page's rail (`ActivityFilters`), folded away on a one-pane
screen.

| Filter                                                             | Preference                                                                                  | How it is read                                |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- | --------------------------------------------- |
| Each deployment, and within it the reader's DMs and each community | `ACTIVITY_HIDDEN`, a device preference of what is hidden, so a community joined later shows | `readFilter` turns it into each read's filter |

`readFilter`:

- does not ask a deployment wholly hidden;
- asks a deployment whose communities are all hidden but whose DMs show for `NO_COMMUNITY`, a
  community no one belongs to;
- for the `dms` view names no community, and does not ask a deployment whose DMs are hidden.
