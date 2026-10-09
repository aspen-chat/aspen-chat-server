# Saved messages and activity

Two pages of the reader's own, across every deployment they use (`useSources`).

## Where it lives

| Part | Where |
| --- | --- |
| Rail links | `CommunityRail`, `railLinkClass` |
| Page layout | `PersonalPage` (`src/features/activity`), `RailPage` (`src/features/layout`) |
| A listed message | `ListedMessage` (`src/features/messages`), `useMessagePlace`, `useMessageOnDemand` |
| Saved messages | `SavedScreen`, `SaveButton`, `SavedMark`, `RecordStore.saves` |
| The activity feed | `ActivityScreen`, `ActivityFilters`, `readFilter` |

## Opening the pages

They open from the top of the community rail (`CommunityRail`), in this order:

1. Activity;
2. Saved messages;
3. the DMs.

Each is a round link drawn alike (`railLinkClass`), ringed while its page is open.

| Page | Route |
| --- | --- |
| Activity | `/activity` |
| Saved messages | `/saved` |

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

A message the server refuses or no longer has is marked missing in the store (`MissingKind`
`message`), and its row leaves the list. So a message deleted, or a channel lost, takes its rows
with it.

## Saved messages

### Store state

| State | Topic | Holds |
| --- | --- | --- |
| `RecordStore.saves` | `saved` | Saves, newest save first |
| `isSaved` | `saved:<messageId>` | Whether one message is saved |

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

  | Screen | Message | Bookmark |
  | --- | --- | --- |
  | Not touch | Any | After whatever it ends with, kept with a grouped message's time |
  | Touch | With a header | After the time in the message's header |
  | Touch | Grouped (shows no time) | After whatever it ends with (`MessageBody`'s `savedMark`) |

### `SavedScreen`

1. It merges every deployment's saves by their ids. They are UUIDv7s, which compare by time
   across deployments.
2. It shows `SAVED_PAGE` at a time, with Show more.
3. It reads each deployment's messages a page at a time (`AspenSync.loadSavedMessages`), as far
   as its saves among those shown.

Each row removes its save.

## The activity feed

`ActivityScreen` shows what tells the reader.

### Reading

1. It reads each deployment's page (`AspenSync.readActivity`, `ACTIVITY_PAGE` at a time), with
   read positions sideloaded.
2. It merges them as search does (`mergeResults`).
3. It reads older pages on request.

What `AspenSync.onNotify` announces while the feed is open, by the same rule, joins the top of
its deployment's feed if the filter shows it.

### Unread and replying

- A message after the reader's position where it was said carries an unread dot. The position is
  a channel's, or a thread's own (`useLastRead`).
- Opening the feed marks nothing read.
- A reply in a thread has a Reply button. It opens the thread's `Composer` in the row, so the
  reader replies without leaving.

### Filters

What the feed shows is chosen in its rail (`ActivityFilters`), folded away on a one-pane screen.

| Filter | Preference | How it is read |
| --- | --- | --- |
| Each deployment, and within it the reader's DMs and each community | `ACTIVITY_HIDDEN`, a device preference of what is hidden, so a community joined later shows | `readFilter` turns it into each read's filter |
| Unread only | `ACTIVITY_UNREAD_ONLY`, a device preference | Reads again with `filter[unread]` |

`readFilter`:

- does not ask a deployment wholly hidden;
- asks a deployment whose communities are all hidden but whose DMs show for `NO_COMMUNITY`, a
  community no one belongs to.
