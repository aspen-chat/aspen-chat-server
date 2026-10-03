# Threads, DMs, and the system account

- Threads are channels of type `thread`, so a thread's history is a window like any channel's
  and its replies arrive as ordinary message events. A message that started one names it in
  `thread`, and the thread record carries `replyCount` and `lastReplyAt`, which
  `MessageItem` shows as a summary under the starter; message reads sideload `threads` for
  those summaries. "Reply in thread" calls `AspenSync.openThread`, which the server answers with
  the thread it makes the first time, and routes to `.../threads/{thread}`, where
  `ChannelScreen` shows `ThreadPanel` (`src/features/threads`) beside the channel, in place of
  it on small screens: the starter, heading the replies in one `MessageList` (its `start`, shown
  once the window reaches the thread's beginning) so the two scroll together however long the
  starter is, and a `Composer` whose `echoTarget` offers to
  also show the reply in the parent channel (`echoToParent`). Messages in a thread cannot start
  threads. An echo is a message of kind `threadEcho` naming the reply in `echoOf`; it is drawn
  from the reply record itself (sideloaded with `echoes`, or fetched with
  `AspenSync.loadMessage`), so the reply's edits show in it. `RecordStore.channels` leaves
  threads out of their community's list though they record its id. A thread's own link, like
  any channel link that names one, redirects to it open beside its parent.
- DMs and group DMs are channels of type `dm` and `groupDm` with no community and their people
  in `recipients`. `AspenSync` reads them at bootstrap (`GET /users/@me/dms`, with their
  people) into `RecordStore.dms()` (topic `dms`): the server's order, most recently active
  first, with any DM that sees a message or is made afterwards moved to the top. A DM whose
  update no longer lists the caller is one they left, and the store drops it with its history;
  `channelRemoved(id)` then tells a screen still showing it that it is gone, not unread, so it
  is not fetched again. `/dms` is `DmLayout` (`src/features/dms`): the DM list beside the
  route's content, with "New message" opening `PeoplePicker`, which lists `RecordStore.people()`
  (everyone in the caller's communities' member lists, since a DM needs a shared community).
  A profile card's Message button opens the one-to-one DM (`AspenSync.openDm`). `DmHeader`
  titles a DM with the other people's names, each a button that opens that person's card, and,
  for a group, offers `addDmRecipient` and `leaveDm`. The list row of the one-to-one DM already
  open is a button to the other person's card, beside the row, rather than a link to where the
  reader already is; either way an unwanted conversation is a press away from a block. `ChannelScreen` serves DMs and community channels alike.
- The system account (`User.system`, the deployment's own, which sends notices) is marked by
  `SystemBadge` beside `BotBadge`, and its card offers no way to message, call, or block it.
  Its DM is read-only: `RecordStore.channelAccess` gives only View channel there, as in a
  blocked DM (`systemDmPeer`, topic `channelAccess:<channelId>`, touched when the account's
  record arrives), and the Composer shows a note in place of the box.
