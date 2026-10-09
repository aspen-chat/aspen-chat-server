# Threads, DMs, and the system account

## Where it lives

| Part | Where |
| --- | --- |
| The thread panel | `ThreadPanel` (`src/features/threads`), shown by `ChannelScreen` |
| Starting a thread | `NewThreadPanel`, `AspenSync.replyInThread` |
| Following | `FollowButton`, `AspenSync.setFollowing` |
| Echoes | `EchoReplyButton`, `AspenSync.echoReply`, `echoToParent` |
| DMs | `DmLayout` (`src/features/dms`), `PeoplePicker`, `DmHeader` |
| The system account | `SystemBadge`, `systemDmPeer` |

## Threads

Threads are channels of type `thread`. A thread's history is a window like any channel's, and its
replies arrive as ordinary message events.

### Summaries

- A message that started a thread names it in `thread`.
- The thread record carries `replyCount` and `lastReplyAt`. `MessageItem` shows them as a summary
  under the starter.
- Message reads sideload `threads` for those summaries.

### Opening a thread

"Reply in thread" on a message with a thread:

1. Routes to `.../threads/{thread}`.
2. `ChannelScreen` shows `ThreadPanel` beside the channel, or in place of it on small screens.

### Starting a thread

"Reply in thread" on a message with none asks the server for nothing:

1. It routes to `.../messages/{message}/thread`.
2. `NewThreadPanel` shows the starter and a `Composer` with `startsThreadOf`. That composer:
   - keeps its draft under the starter;
   - tells no one the caller is typing;
   - offers no polls.
3. Its first reply goes through `AspenSync.replyInThread`
   (`POST /messages/{message}/thread/messages`). That makes the thread with the reply, posted or
   held, and sets the starter's `thread` at once.
4. A starter that names a thread, by that reply or by anyone's, takes the panel to the thread
   itself.

Special cases:

- A command sent as the first reply is invoked in a thread `AspenSync.openThread` makes first.
  **Why:** commands are checked against the channel they run in.
- A first reply held for its previews makes the thread as it is held.
- A held first reply that is dropped takes the thread with it when nothing else is there.
  `ThreadPanel`, seeing its thread removed and its starter naming none, gives way to
  `NewThreadPanel`. Its composer shows the dropped reply to be sent again (see
  [Composer and drafts](composer-and-drafts.md)).

### The thread panel

`ThreadPanel` holds:

- the starter, heading the replies in one `MessageList` (its `start`, shown once the window
  reaches the thread's beginning), so the two scroll together however long the starter is;
- a thread shorter than the panel standing at its top (`fromTop`, its skeleton too), rather than
  at its bottom by the composer as a channel's history does;
- "No replies in this thread." under the starter while the thread holds none, as when every reply
  is deleted (the list's `empty`);
- a `Composer` whose `echoTarget` offers to also show the reply in the parent channel
  (`echoToParent`).

Its list marks what is read there, as a channel's does.

### Following

The panel header's bell (`FollowButton`, a toggle) follows the thread or stops following it
(`AspenSync.setFollowing`, which the store follows at once).

- Following means every reply tells the reader.
- Taking part follows it too, as the server announces.

### Echoes

An echo shows a thread reply in the parent channel too.

A reply sent without one offers its author "Also send to …" among its actions (`EchoReplyButton`)
while:

- its `echo` is empty; and
- they may send messages there.

`AspenSync.echoReply`:

1. makes the echo;
2. caches it;
3. sets the reply's `echo` at once.

- A toast names where it went, since on a phone the parent channel is not on screen.
- The reply's `update` events keep `echo` current, so deleting the echo offers the action again.
- An echo is a message of kind `threadEcho`, naming the reply in `echoOf`.
- It is drawn from the reply record itself (sideloaded with `echoes`, or fetched with
  `AspenSync.loadMessage`), so the reply's edits show in it.

### Rules

- Messages in a thread cannot start threads.
- `RecordStore.channels` leaves threads out of their community's list, though they record its id.
- A thread's own link, like any channel link that names one, redirects to it open beside its
  parent.

## DMs and group DMs

DMs and group DMs are channels of type `dm` and `groupDm`, with no community and their people in
`recipients`.

### The list

`AspenSync` reads them at bootstrap (`GET /users/@me/dms`, with their people) into
`RecordStore.dms()` (topic `dms`).

- The order is the server's: most recently active first.
- Any DM that sees a message, or is made afterwards, moves to the top.

The list comes a page at a time (`DM_PAGE`, 100):

1. Bootstrap reads the first page (`setDms`), with every DM's mutes and notification settings
   whole.
2. As the end of the DM list (`OlderDms`) comes into view, it reads the next page of each
   deployment with more (`AspenSync.loadMoreDms`, `appendDms`).

### Leaving

- A DM whose update no longer lists the caller is one they left. The store drops it with its
  history.
- `channelRemoved(id)` then tells a screen still showing it that it is gone, not unread, so it is
  not fetched again.

### Screens

`/dms` is `DmLayout` (`src/features/dms`): the DM list beside the route's content.

- "New message" opens `PeoplePicker`. It lists `RecordStore.people()`: everyone in the caller's
  communities' member lists, since a DM needs a shared community.
- A profile card's Message button opens the one-to-one DM (`AspenSync.openDm`).
- `DmHeader` titles a DM with the other people's names, each a button that opens that person's
  card. For a group it offers `addDmRecipient` and `leaveDm`.
- The list row of the one-to-one DM already open is a button to the other person's card, beside
  the row, rather than a link to where the reader already is.
- Either way an unwanted conversation is a press away from a block.
- `ChannelScreen` serves DMs and community channels alike.

## The system account

The system account (`User.system`) is the deployment's own, and sends notices.

- `SystemBadge` marks it, beside `BotBadge`.
- Its card offers no way to message, call, or block it.
- Its DM is read-only. `RecordStore.channelAccess` gives only View channel there, as in a blocked
  DM (`systemDmPeer`, topic `channelAccess:<channelId>`, touched when the account's record
  arrives).
- The Composer shows a note in place of the box.
