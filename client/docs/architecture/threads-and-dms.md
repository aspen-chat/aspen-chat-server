# Threads, DMs, and the system account

- Threads are channels of type `thread`, so a thread's history is a window like any channel's
  and its replies arrive as ordinary message events. A message that started one names it in
  `thread`, and the thread record carries `replyCount` and `lastReplyAt`, which
  `MessageItem` shows as a summary under the starter; message reads sideload `threads` for
  those summaries. "Reply in thread" on a message with a thread routes to
  `.../threads/{thread}`, where `ChannelScreen` shows `ThreadPanel` (`src/features/threads`)
  beside the channel, in place of it on small screens. On one with none it asks the server for
  nothing: it routes to `.../messages/{message}/thread`, where `NewThreadPanel` shows the
  starter and a `Composer` with `startsThreadOf`, which keeps its draft under the starter, tells
  no one the caller is typing, and offers no polls. Its first reply goes through
  `AspenSync.replyInThread` (`POST /messages/{message}/thread/messages`), which makes the
  thread with the reply, posted or held, and sets the starter's `thread` at once; a starter
  that names a thread, by that reply or by anyone's, takes the panel to the thread itself. A
  command sent as the first reply is invoked in a thread `AspenSync.openThread` makes first,
  since commands are checked against the channel they run in. A first reply held for its
  previews makes the thread as it is held; dropped, it takes the thread with it when nothing
  else is there, and `ThreadPanel`, seeing its thread removed and its starter naming none, gives
  way to `NewThreadPanel`, whose composer shows the dropped reply to be sent again (see Composer
  and drafts). `ThreadPanel` shows the starter, heading the replies in one `MessageList` (its `start`, shown
  once the window reaches the thread's beginning) so the two scroll together however long the
  starter is, a thread shorter than the panel standing at its top (`fromTop`, its skeleton
  too) rather than at its bottom by the composer as a channel's history does, "No replies in
  this thread." under the starter while the thread holds none, as when every reply is deleted
  (the list's `empty`), and a `Composer` whose `echoTarget` offers to
  also show the reply in the parent channel (`echoToParent`). Its header's bell (`FollowButton`,
  a toggle) follows the thread or stops following it (`AspenSync.setFollowing`, which the store
  follows at once), so every reply tells the reader; taking part follows it too, as the server
  announces. Its list marks what is read there, as a channel's does. A reply sent without one offers its
  author "Also send to …" among its actions (`EchoReplyButton`) while its `echo` is empty and
  they may send messages there; `AspenSync.echoReply` makes the echo, caches it, and sets the
  reply's `echo` at once, and a toast names where it went, since on a phone the parent channel
  is not on screen. The reply's `update` events keep `echo` current, so deleting the echo offers
  the action again. Messages in a thread cannot start threads. An echo is a message of kind `threadEcho` naming the reply in `echoOf`; it is drawn
  from the reply record itself (sideloaded with `echoes`, or fetched with
  `AspenSync.loadMessage`), so the reply's edits show in it. `RecordStore.channels` leaves
  threads out of their community's list though they record its id. A thread's own link, like
  any channel link that names one, redirects to it open beside its parent.
- DMs and group DMs are channels of type `dm` and `groupDm` with no community and their people
  in `recipients`. `AspenSync` reads them at bootstrap (`GET /users/@me/dms`, with their
  people) into `RecordStore.dms()` (topic `dms`): the server's order, most recently active
  first, with any DM that sees a message or is made afterwards moved to the top. The list comes
  a page at a time (`DM_PAGE`, 100): bootstrap reads the first (`setDms`), with every DM's mutes
  and notification settings whole, and the end of the DM list (`OlderDms`) reads the next of
  each deployment with more (`AspenSync.loadMoreDms`, `appendDms`) as it comes into view. A DM whose
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
