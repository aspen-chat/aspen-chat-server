# Threads and DMs: design notes

Why threads, DMs, and the system account work as they do. The how is in the [main pages](index.md).

## Threads

See [Threads](threads.md).

- **A thread is made by its first reply, in the transaction that posts it.** Replying is what starts a thread, and a thread starts with its reply.
- **The first reply is checked in full before the transaction opens.** So a refused reply announces no thread. The transaction checks it all again.
- **A thread whose first reply is dropped, with nothing ever posted or waiting in it, is removed.** So the next first reply makes a new one.
- **Every posting holds its channel's row `FOR KEY SHARE` from its check until it commits.** So a reply racing the removal of an unreplied thread is either seen by it or finds the thread gone.
- **Both ends of a thread are written in one transaction, under a lock on the starter.** So two first replies or openings make one thread.
- **A community's thread records the community.** So routing and history reads treat it like any channel.
- **`replyCount` and `lastReplyAt` are kept under the thread row's lock.** So they stay exact.

## Echoes

See [Echoes](echoes.md).

- **An echo is a reference, not a copy.** The client shows the reply's current content, and the echo's content cannot be edited.
- **Deleting a reply announces its echo's deletion first.** So no client ever holds an echo whose reply is gone.
- **The reply's `echo` reference is checked at commit.** The echo is inserted after the reply that names it.
- **An echo is read, routed, and searched as a message of the parent channel.** Echoing shows no one anything new: the echo shows a reply whose thread its readers already read.

## Following threads

See [Following threads](following-threads.md).

- **A follow of a thread whose parent the user loses stays, but tells nothing.** Each route that would tell (push, the feed, the event stream, the list) checks whether they may read it now, so the follow tells them again once they may.

## DM access

See [DMs and group DMs](dms.md).

- **`channel.dm_key` holds the pair's sorted ids under a unique index.** So concurrent first messages make one DM.
- **No role reaches into a DM.** Only recipients read or write it.
- **Moderate any community reaches a DM only through `channel_access_reading` and `channel_access_moderating`, which log.** Every other path, search included, finds no DM a moderator is not in. So a new path cannot read a DM unlogged by forgetting to log.
- **The reading log is written before anything is read, once per DM.** However many of its messages a read returns.

## Listing DMs

See [Listing them](dms.md#listing-them).

- **`include=mutes` and `include=notifications` return every DM's rows, not just the page's.** They are the caller's own few rows, and a DM not listed yet must be known muted when it is heard from.
- **Activity is kept per person in `dm_recipient.active_at`, moved on by the `message_inserted` trigger.** So the list is read through `dm_recipient_by_activity` however many DMs someone has.

## Calls in DMs

See [Calls in DMs](dm-calls.md).

- **No one moderates a DM's call.** Moderating a call takes Manage calls, a community permission no one holds in a DM.
- **A ring that runs out ends by every client's clock, with no event.** The reaper only clears the rows, and reads never return a spent one.
- **A replacing session keeps its start, starter, and `had_company`.** So its people rejoining rings no one again.
- **`call` and `missed_call` messages wake no phone, notify no one, and are left out of search.** The ring already told everyone of the call.

## The system account

See [The system account](system-account.md).

- **Its username is outside `user_name_key` and every lookup by name.** So it takes no one's name.
- **It has no password or token.** So nothing signs in as it.
- **Names confusable with it are refused.** So nobody passes for it.
- **Notices are ordinary DM messages.** So push, unread counts, and search treat them like any DM.
- **A notice names `@everyone` only as code.** The account holds every permission in its DM, so a tag there would count.
- **Nothing in its messages is fetched for a link preview or shown as a link.** Notices quote names others chose (a community's, the person whose account was deleted).
