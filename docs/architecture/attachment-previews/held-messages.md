# Held messages

A message sent with an attachment whose preview is still being made can be held until the preview is
ready (`app::message::held`). [Why](design-notes.md#held-messages)

Held messages also hold what waits for a busy channel's mana: messages, slash commands, edits,
polls, and echoes (`held_message.waiting` is `mana`; see
[Channel mana](../channel-mana/index.md#waiting-rows)). This page is about waiting for previews;
both kinds are listed, cancelled, and announced alike.

A message sent while its files are still uploading is a held message as well (`waiting` is
`uploads`; see [Sent before its files arrive](sent-before-uploaded.md)). Once each file is confirmed
it waits for its previews as described here, whether it is a message or a slash command.

A message scheduled for later is a held message too (`waiting` is `scheduled`, `send_at` its time;
see [Scheduled messages](../scheduled-messages.md)). It is posted by the same job, which comes due
at its time, but it is listed apart (`GET /users/@me/held-messages` leaves it out), keeps an order
of its own, and is kept for its author when it cannot be posted, where what is described here is
dropped.

## Holding a message

1. A client that shows its sender a message waiting sends `mayHold: true` on
   `POST /channels/{channel}/messages`.
2. The message is checked as any message is (`app::message::check_posting`).
3. It is kept in `held_message` with the language it was sent in.
4. The answer is `202 Accepted` with the held message: its id, channel, content, attachments, and
   when it was held.

Without `mayHold`, as older clients and other deployments' send, a message is posted at once.

## How long it is held

Each preview job holds the messages its attachment is in until its `holdUntil`. That is twenty
seconds after the upload (`preview::HOLD`), or earlier: when the preview is made, found not worth
keeping, or fails, `holdUntil` is set to now.

## Releasing it

Each held message is posted by a job of its own, saved with it: `releaseHeldMessage`, interactive,
keyed by the held message (`held::release_step`; see [Jobs](../jobs/index.md)).

- A preview job that settles makes the jobs of the messages it held due at once, in the transaction
  that records it (`held::wake_holding`, through the index `held_message_attachments_idx`).
- A job that finds its message still held looks again when the hold runs out, and at least every
  five seconds (`held::LOOK_EVERY`).
- A job whose author sent a message before it that is still held waits a second. So an author's held
  messages are posted in the order they were sent (through the index `held_message_by_author`).
  Scheduled messages are not waited for, nor are those still waiting for their files.

Once nothing holds it, the job:

1. Asks the channel's mana, as posting does. When the channel is busy, the row becomes one waiting
   for mana (`mana::queue::wait_held`), and the job is done.
2. Posts the message as it was sent (`app::message::post`), in its language, with the author's
   permissions as they are then and the channel's plugins deciding it then.
3. Deletes the held row in the same transaction (`held::take`), so it is posted once however its job
   is run.
4. Tells the author's apps which message it became, with `heldMessagePosted`.

A preview finished after the hold ran out reaches the message's readers by `attachmentPreviewed`.

## When posting fails

| Case                                                                                                                                      | What happens                                                                                                                                                                                                                                                                                     |
| ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Can no longer be posted (the author lost the right to post there, was removed, or the channel went)                                       | Dropped. `heldMessageFailed` tells the author's apps why, in the language it was sent in.                                                                                                                                                                                                        |
| Fails for a reason that may pass                                                                                                          | Tried again every thirty seconds, ten times (`held::MAX_ATTEMPTS`), and dropped as above on the last.                                                                                                                                                                                            |
| Author left, was removed, or was banned from the community, left the group DM, or was banned from the deployment or deleted their account | Dropped at once, everything they held there (in a community, its channels and threads; on a deployment ban or deletion, everywhere): the change saves a `dropHeldBy` job in its transaction (`held::drop_held_by`), which drops it a hundred at a time, each told why (`heldNoLongerPermitted`). |
| Channel deleted                                                                                                                           | Dropped when it would be posted, since posting is checked then.                                                                                                                                                                                                                                  |

The row goes with its user or channel only when those rows themselves are removed.

## Held first replies

A held first reply makes its thread as it is held. Dropping it removes the thread when nothing else
was ever posted or waits there (see
[Threads](../threads-and-dms/threads.md#when-the-first-reply-is-dropped)).

## Cancelling one

`DELETE /users/@me/held-messages/{held}` cancels a held message of the caller's that has not been
posted, whatever it waits for (`held::cancel`).

- The row goes in one transaction with `heldMessageCancelled`, user-scoped, which tells the author's
  other apps.
- A first reply that made its thread as it was held (`made_thread`) takes the thread with it, as a
  dropped one does; any other held message leaves its thread alone.
- It is not found when it was never the caller's, or has been posted.

## Listing

`GET /users/@me/held-messages` lists the caller's held messages, whatever each waits for but a time
it was scheduled for, in the order of their ids, which are UUIDv7 made as each was held.

- It answers a page of at most `LIST_PAGE` (100) at a time (through the index `held_message_list`),
  as `{data, nextCursor}`: `nextCursor` is the `cursor` of the page after, and `null` on the last.
  The read asks for one held message more than the page holds and gives a cursor only when it gets
  it, so a reader goes by `nextCursor` and never by how many a page holds.
- A cursor (`app::cursor`) is the id of the page's last held message, which is all the list is
  ordered by. It is compared as an id alone, so the page after it is read whether or not that
  message is still held: it may have been posted, dropped, or cancelled since the page before. Text
  that is not such a cursor, or is a cursor of another list (each names its own), is refused with
  `validation`.
- Their apps read it whole as they start, a page after each cursor until an answer gives none.

## Order

- A held message is posted after messages sent after it without a hold.
- Held messages of one author go in the order they were sent.
