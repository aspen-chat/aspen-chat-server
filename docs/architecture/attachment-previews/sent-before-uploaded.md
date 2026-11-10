# Sent before its files arrive

A message can be sent while its files are still being uploaded (`app::message::held`). The server
holds it, and posts it once every file has been confirmed, so its author need not wait for an upload
to press Send. [Why](design-notes.md#sent-before-its-files-arrive)

It is a held message ([Held messages](held-messages.md)) that waits for its uploads
(`held_message.waiting` is `uploads`) before it waits for anything else. This page says what is its
own.

## What can be

| What                                     | Sent with `awaitUploads: true` on                                               |
| ---------------------------------------- | ------------------------------------------------------------------------------- |
| A message, a reply, a thread's first one | `POST /channels/{channel}/messages`, `POST /messages/{message}/thread/messages` |
| A slash command that takes a file        | `POST /channels/{channel}/commands`, beside the invocation                      |
| A message scheduled for later            | `POST /channels/{channel}/scheduled-messages`                                   |

Without `awaitUploads`, as clients that do not show a message waiting send, an attachment not yet
confirmed is refused (`attachmentNotReady`), as it always was.

## Holding it

1. `attachments` may name reservations of the caller's that are not yet confirmed
   (`message::Uploads::MayBePending`). Each must still be theirs and in no message.
2. Everything else is checked as it would be to post it: the right to send there and to attach
   files, what it replies to, a command's arguments (`message::check_posting_awaiting`,
   `bot_command::check`). A first reply makes its thread as it is held.
3. The row is saved with `waiting` `uploads` and `uploads_due`, a day after it was sent
   (`held::UPLOADS_WAIT`), and its `releaseHeldMessage` job is due then.
4. The answer is `202 Accepted` with the held message, whose `awaitedUploads` is true, and
   `heldMessageChanged` tells the author's other devices.

A scheduled one is answered `201` as any is, with `status` `uploading`, and told by
`scheduledMessageChanged`.

One author has at most fifty messages and commands waiting for their files at once
(`held::MAX_AWAITING_UPLOADS`), scheduled messages apart, which keep their own limit of a hundred.
The fifty-first is refused with `validation`. The count is taken holding the author's own row, so
two requests at once cannot both find room.

### A file confirmed as it is sent

The request looks first, with nothing locked, at whether any file is unconfirmed, which decides the
way it is checked. Holding it, it locks each attachment row against being confirmed (`FOR SHARE`,
`message::ensure_attachments_ready`) until the held row is saved. So a confirmation at that moment
either finished first, and is seen, or waits and then finds the held row. A message whose last file
was confirmed in between is saved waiting for its previews, with its job due at once.

## When the last file is confirmed

`POST /attachments/{attachment}/confirm` sets the message going in its own transaction
(`held::uploaded`):

1. It locks every held row that waits for uploads and names the attachment, through
   `held_message_attachments_idx`, in a statement of its own.
2. Each whose files are now all confirmed stops waiting for uploads (`held::advance`): `waiting`
   becomes `preview`, or `scheduled` for one with a time, and its job comes due now, or at its time.
3. The author's devices are told: `heldMessageChanged`, or `scheduledMessageChanged`.
4. The job runners are woken as the request ends.

From there it is a held message like any other. A picture just confirmed has a preview being made,
which holds the message as it holds one sent after the upload; then it is posted, or waits for its
channel's mana.

Two requests confirming the last two files of one message at once both lock the row before either
looks at the files, each in a new statement. The second to take the lock looks after the first has
committed, and so sees its file.

`uploads_due` stays set once the files are there. It marks the row as one that waited for its
uploads (`awaitedUploads`), which its author's apps go on showing where it will be posted until it
is.

## Order

A message waiting for its files keeps nothing else waiting (`held::behind_another`): what its author
sends after it is posted ahead of it. Once its files are there it takes its place among their held
messages by when it was sent.

## When the files never arrive

| Case                                                        | What happens                                                                                               |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| A day passes (`uploads_due`)                                | Its job comes due and fails it (`held::await_uploads`), saying its files were not uploaded in time.        |
| One of its files is deleted before it is confirmed          | `DELETE /attachments/{attachment}` brings its job forward (`held::wake_awaiting`), which fails it at once. |
| Its author cancels it (`DELETE .../held-messages/{held}`)   | Gone, with its job. Its files are theirs to send another way or delete.                                    |
| Its author loses the right to post there before they arrive | Nothing yet. It is refused when it would be posted, as any held message is.                                |
| Its author leaves, is removed, or is banned                 | Dropped with everything they hold there (`held::drop_held_by`).                                            |

Failing it drops it and tells its author why (`heldMessageFailed`). A scheduled one is kept, failed,
for its author while they can still see where it would go, as a scheduled message that cannot be
posted is ([Scheduled messages](../scheduled-messages.md#when-it-cannot-be-posted)).

A look of the job that finds every file confirmed after all sets the message going, as the
confirmation would have.

## Sending a file again

An upload that failed, or whose link expired before it began, is given another link for the same
attachment: `POST /attachments/{attachment}/renew` with the file's `byteSize`
(`attachment::renew_upload`). The attachment keeps its id, so the message waiting for it needs no
change. The bytes count toward the uploader's quota again, since nothing records how large the
reservation was ([Uploads](uploads.md#upload-quota)). A confirmed attachment, or another's, is not
found.

## Scheduled

A message scheduled while its files upload waits for them first, whatever its time: `status` is
`uploading`. Given another time meanwhile, or sent now, it takes the time and goes on waiting for
them (`scheduled::set_time`). Once they are there it is `scheduled`, or posted at once if its time
has come.

## Events

All to the author alone (`EventScope::User`), in the transaction that makes the change:

| Event                     | When                                                                                             |
| ------------------------- | ------------------------------------------------------------------------------------------------ |
| `heldMessageChanged`      | It is held for its uploads; the last is confirmed; it then begins waiting for its channel's mana |
| `scheduledMessageChanged` | A scheduled one, at the same moments, and when it fails                                          |
| `heldMessagePosted`       | It was posted, naming the message                                                                |
| `heldMessageCancelled`    | Its author cancelled it                                                                          |
| `heldMessageFailed`       | It was dropped, with why                                                                         |

## What removes rows

- A row goes when it is posted, cancelled, or failed. None waits for its files more than a day.
- One author keeps at most fifty, and a hundred scheduled.
- Its job goes with it (`held::forget_job`), rather than staying until the day is out to find
  nothing.
- A reservation never confirmed stays as every unconfirmed reservation does
  ([Unsent attachments](unsent-attachments.md)); its staged bytes are swept with the other uploads.

## When access is given or taken away

1. **Who can observe it, and by which routes?** Only its author: the `202`,
   `GET /users/@me/held-messages` (or `GET /users/@me/scheduled-messages`), and its events on their
   own subject. No push, no search, no link. Its files are reservations only their uploader can
   confirm, read, or delete. Once posted it is a message like any other.
2. **What decides it, and where is that checked?** Holding it takes what posting takes
   (`message::check_posting_awaiting`, whose `channel_access` proves the author may send there).
   Posting it checks again, in the posting's transaction, with the author's permissions of that
   moment.
3. **When the deciding permission is lost?**
   - Losing the right to send (a role's permissions, a role taken or deleted, an override, a move, a
     category deleted) leaves the row. When its files arrive it is refused and dropped, its author
     told why; a scheduled one is kept failed, or dropped if its author can no longer see the
     channel.
   - Leaving, removal, a ban, a group DM left, a ban from the deployment, and an account deleted
     drop everything the author holds there by a `dropHeldBy` job saved in the change's transaction,
     these among it.
   - A sign-out or a password change ends the session, not the message, which is theirs, sent while
     signed in. Confirming its files takes a session of its author's, though, and only the device
     sending them can: one signed out while it uploads stops, and the message fails when the day is
     out, unless its author cancels it sooner from another device.
   - The client's store holds only its author's own, and lets each go by the events above.
4. **When it is gained?** Nothing is shown to anyone new.
5. **Does every path that changes it announce it?** Holding it, the last confirmation, joining the
   mana queue, failing, posting, cancelling, and dropping each publish. A cascade (the channel or
   the account removed) does not: the channel's removal takes it from nowhere the client shows it,
   and a deleted account has no client.
6. **Is it published inside the transaction that makes the change?** Yes; a rollback is answered by
   `userResync`.

## What a client already holds

1. **Who holds this record?** Its author's apps: from the answer to the request on the device that
   sent it, from `heldMessageChanged` on their others, and from the bootstrap read.
2. **Is every field changed only through an event?** `waiting` changes by `heldMessageChanged`,
   which carries the whole record, from the confirmation, the job, and the mana queue alike. A
   scheduled one's `status`, `sendAt`, and `failure` change by `scheduledMessageChanged`. Nothing
   else of it changes.
3. **Is the caller's own state announced to their other devices?** Every event goes to the user's
   subject. The device that sent it keeps the answer only where the stream has not told of the
   message yet (`RecordStore.putHeldMessage`), and leaves later changes to the events.
4. **Counts?** None is kept.
5. **What does a REST read overwrite?** The lists are read only at bootstrap, whose events are held
   and applied after it. An answer that arrives after the message's posting or cancellation was told
   does not bring it back (`#settledHeld`).
6. **What does the client infer from a parent's deletion?** Nothing: a deleted channel's held
   messages are dropped as each would be posted, and told.
7. **What does a catch-up leave out?** Nothing of its own; user-scoped events are replayed whole.
8. **Outside the stream?** Which of its files are still on their way is known only to the device
   sending them. Another device shows the message waiting, and learns that they arrived when
   `waiting` changes.

## Testing

`scripts/check_permissions.py`, `speculative_sends`: two of the author's devices and a bystander
across holding, each file's confirmation (one, two, and six at once), a picture's preview, a
cancellation, a file deleted, another link for an upload, a scheduled message, a slash command, the
wait giving out, the right to post lost meanwhile, the limit, and removal.
