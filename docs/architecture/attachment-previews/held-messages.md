# Held messages

A message sent with an attachment whose preview is still being made can be held until the preview is ready (`app::message::held`). [Why](design-notes.md#held-messages)

## Holding a message

1. A client that shows its sender a message waiting sends `mayHold: true` on `POST /channels/{channel}/messages`.
2. The message is checked as any message is (`app::message::check_posting`).
3. It is kept in `held_message` with the language it was sent in.
4. The answer is `202 Accepted` with the held message: its id, channel, content, attachments, and when it was held.

Without `mayHold`, as older clients and other deployments' send, a message is posted at once.

## How long it is held

Each preview job holds the messages its attachment is in until its `holdUntil`. That is twenty seconds after the upload (`preview::HOLD`), or earlier: when the preview is made, found not worth keeping, or fails, `holdUntil` is set to now.

## Releasing it

Each held message is posted by a job of its own, saved with it: `releaseHeldMessage`, interactive, keyed by the held message (`held::release_step`; see [Jobs](../jobs/index.md)).

- A preview job that settles makes the jobs of the messages it held due at once, in the transaction that records it (`held::wake_holding`, through the index `held_message_attachments_idx`).
- A job that finds its message still held looks again when the hold runs out, and at least every five seconds (`held::LOOK_EVERY`).
- A job whose author sent a message before it that is still held waits a second. So an author's held messages are posted in the order they were sent (through the index `held_message_by_author`).

Once nothing holds it, the job:

1. Posts the message as it was sent (`app::message::post`), in its language, with the author's permissions as they are then and the channel's plugins deciding it then.
2. Deletes the held row in the same transaction, so it is posted once however its job is run.
3. Tells the author's apps which message it became, with `heldMessagePosted`.

A preview finished after the hold ran out reaches the message's readers by `attachmentPreviewed`.

## When posting fails

| Case | What happens |
| --- | --- |
| Can no longer be posted (the author lost the right to post there, was removed, or the channel went) | Dropped. `heldMessageFailed` tells the author's apps why, in the language it was sent in. |
| Fails for a reason that may pass | Tried again every thirty seconds, ten times (`held::MAX_ATTEMPTS`), and dropped as above on the last. |
| Author's account or channel deleted | Dropped when it would be posted, since posting is checked then. |

The row goes with its user or channel only when those rows themselves are removed.

## Held first replies

A held first reply makes its thread as it is held. Dropping it removes the thread when nothing else was ever posted or waits there (see [Threads](../threads-and-dms/threads.md#when-the-first-reply-is-dropped)).

## Listing

`GET /users/@me/held-messages` lists the caller's held messages, oldest first.

- It answers a page of at most `LIST_PAGE` (100) at a time, after `after` (through `held_message_by_author`).
- Their apps read it whole as they start.

## Order

- A held message is posted after messages sent after it without a hold.
- Held messages of one author go in the order they were sent.
