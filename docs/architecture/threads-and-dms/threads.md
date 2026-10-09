# Threads

A thread is a channel of type `thread` (`app::thread`). It holds replies to one message of a text channel, DM, or group DM.

## Making a thread

A thread is made by its first reply, in the transaction that posts it. Replying is what starts a thread, and a thread starts with its reply.

| How | Endpoint | Answers |
| --- | --- | --- |
| First reply | `POST /messages/{message}/thread/messages` (`app::message::To::ThreadOf`) | As posting does |
| Opening with no reply | `PUT /messages/{message}/thread` (`open_thread`), for clients that open a thread before replying | `201` when it makes it, `200` after |

The first-reply endpoint posts as `POST /channels/{channel}/messages` posts to the thread:

- The same checks, plugins, echo, and holding apply.
- The reply's `channelId` names the thread. Once the thread is made, posting there is the same, so clients post later replies to the thread itself.
- Making the thread takes Start threads, besides Send in threads for the reply.

### Steps of a first reply

1. The thread's id is chosen before the plugins decide the reply. They are told it is in a thread of the parent (`intercept::Draft::unmade_thread_of`).
2. Everything is checked before the transaction opens (`check_first_reply`), so a refused reply announces no thread.
3. The transaction checks it all again, makes the thread, and posts the reply.

A reply held for its previews (see [Attachment previews](../attachment-previews/index.md)) makes the thread as it is held, and waits in it.

### What is written

- The thread's record names its `parentChannel` and `starterMessage`.
- The starter names the thread in `Message.thread`.
- Both ends are written in the transaction that makes the thread, under a lock on the starter. Two first replies or openings make one thread, the later posting to the thread the earlier made.

These cannot start a thread:

- A message in a thread.
- An echo (see [Echoes](echoes.md)).

## When the first reply is dropped

A thread whose first reply is dropped goes with it (`thread::remove_if_unreplied`), in the transaction that drops it, when both hold:

- Nothing was ever posted in the thread (no message, deleted ones included).
- Nothing else waits there. Waiting messages are found by `held_message_by_channel`.

Removing it:

1. Its followers are told they follow it no longer (`threadFollowChanged`).
2. The starter's `thread` is cleared and announced.
3. The thread's `delete` is announced.
4. Its row is deleted, taking its read positions, mutes, and notification settings with it.

The next first reply makes a new thread.

### Locking against a racing reply

- The removal locks the starter, as making a thread does, and then the thread's row.
- Every posting in a channel holds the channel's row from its check until it commits (`message::hold_channel`, `FOR KEY SHARE`), as the message's own reference would from its insert on. Poll creation holds it too.
- A reply racing the removal is either seen by it, keeping the thread, or finds the thread gone and is answered `404`.
- A first reply racing it makes a new thread.

## Threads in a community

A thread in a community records the community too. So routing and history reads treat it like any channel.

Every listing of a community's channels leaves threads out (`parent_channel IS NULL`).

## When the parent goes

A thread goes with its parent. Once the parent channel is deleted, `channel_access` finds neither:

- The thread, its replies, and posting in it are all answered `404`.
- Search reaches threads only through their live parents, so it finds none of them.

## Reply summary

`replyCount` and `lastReplyAt` summarise the thread for its starter.

- Every message posted to the thread counts, polls and poll announcements included.
- They are kept exact under the thread row's lock.
- A change is announced as the thread's `update` event.

## Sideloading

Message reads sideload:

| `include` | Brings | As |
| --- | --- | --- |
| `threads` | The starters' threads | `included.channels` |
| `echoes` | The replies echoes name | `included.messages` |
