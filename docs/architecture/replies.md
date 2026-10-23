# Replies

A message may reply to another message of its channel, as Discord's Reply does: clients show what it
answers above it, with a jump to it, and it may tag that message's author. Threads are another
feature ([Threads and DMs](threads-and-dms/index.md)); a thread is a channel of its own, so a reply
in one answers a message of that thread.

| Part          | Where                                                                                                      |
| ------------- | ---------------------------------------------------------------------------------------------------------- |
| Logic         | `app::message::reply`                                                                                      |
| Stored        | `message.reply_to` (a foreign key, indexed where set) and `message.tags_replied`; on the record, `replyTo` |
| Posting       | `replyTo` and `tagReplied` in `MessageCreateRequest`, on `POST /channels/{channel}/messages`               |
| Held messages | `held_message.reply_to` and `tags_replied`; on the record, `replyTo` and `tagReplied`                      |
| Sideload      | `include=linked`, as for [message links](message-links.md)                                                 |

## Posting a reply

- `replyTo` must name a live message of the channel posted in, checked in the transaction that posts
  (or holds) the reply.
  - A message of another channel, or no message, is refused alike (`validation`, `replyElsewhere`),
    so the refusal tells nothing of channels the author may not read.
  - A deleted one is refused with `replyTargetDeleted`.
  - The first reply of a thread not made yet replies to nothing, since the thread holds nothing.
- A message held for its previews ([Attachment previews](attachment-previews/index.md)) was checked
  when sent, and is posted replying to what it answered even if that was deleted meanwhile, then
  tagging no one for it.
- `replyTo` is permanent: an edit never changes it, and it stays named once the original is deleted
  (deletion is soft).
- Commands, warnings, cards, and the system account's notices are posted through `create_message`
  and reply to nothing.

## Tagging the author

- A reply tags the original's author unless posted with `tagReplied: false` (absent means true), as
  `<@id>` in its text would: it takes Mention members, and the author must belong where the channel
  is ([Tagging](tagging.md)).
- It never tags its own author, nor the author of a deleted original.
- The tag is put first among the text's, within `MAX_TAGS` (`Requested::with_user`).
- `tags_replied` keeps the choice, so an edit that tags afresh tags them again while the original is
  live (`reply::tagged_on_edit`).
- What counts is the message's `mentions` like any tag, so unread counts, notifications, push, and
  the activity feed need nothing of their own.

## Reading

Message reads with `include=linked` read the reply targets with the links
(`app::message_link::read_linked`), as `included.linkedMessages` and `included.messages`:

| `state`     | When                                                          |
| ----------- | ------------------------------------------------------------- |
| `available` | The original is live; its record is among `included.messages` |
| `deleted`   | The original was deleted                                      |

`unavailable` cannot happen for a reply: whoever may read it may read its channel.

## When access is given or taken away

1. **Who observes it.** The reply is a message and goes where its channel's messages go: REST reads,
   the event stream (`EventScope::Channel`, carrying `replyTo` in its `create`), push, search, and
   links to it. The original is read only through the same channel.
2. **What decides it.** The reply's channel, through `channel_access`; since the original is in that
   channel, no second check is made, and posting refuses an original elsewhere.
3. **When access is lost.** The reader loses the reply and the original together, as every message
   of the channel; nothing about the reply is kept apart from them. Losing Mention members stops
   later replies and edits tagging; tags already made stay, as every tag does.
4. **When it is gained.** The channel's messages, replies among them, come as for any channel.
5. **Every path announces it.** Only posting sets a reply, in the transaction that publishes the
   message's `create`; a deleted original is announced by its own `delete`, from which clients infer
   the reply's line.
6. **In the transaction.** Yes, with the message.

`scripts/check_permissions.py` checks these (`replies`).
