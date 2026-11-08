# Unsent attachments

An attachment is meant to be sent. One nobody sends is deleted, so the store is not a file host for
uploads nobody sends.

## The sweep

`attachment::sweep_unsent` deletes an attachment's row, object, and preview when:

- it was confirmed a day ago (`app::attachment::UNSENT_LIFETIME`);
- it has never been in a message; and
- it is not held in one: waiting for its preview, for its channel's mana, or for the time it was
  scheduled for ([Scheduled messages](../scheduled-messages.md)).

It runs as part of the recurring `sweepUploads` job (`media_store::sweep_step`, hourly; see
[Jobs](../jobs/kinds.md#sweepuploads)), 500 at a time under `FOR UPDATE SKIP LOCKED`.

A file in a scheduled message stays unsent, and confirmed over a day ago, for as long as the message
waits: up to two years. So the sweep walks the unsent attachments confirmed a day ago once, oldest
first through `attachment_unsent_idx`, each statement going on from where the last ended, and reads
each such file once a sweep rather than once a batch.

## Scheduled messages deleted

A scheduled message its author deletes, to change it or to send it another way, lets its files go
long after they were confirmed. Each still unsent is counted as confirmed at that moment
(`attachment::renew_unsent`), in the deletion's transaction, so it is kept a day more: the app that
takes the message back into its box holds the files' records, not the files to upload again.

## Sent attachments

- `attachment.sent` records that an attachment has been in a message. It is set as the attachment is
  put in one.
- One an edit has since taken out of its message is not swept.

## Racing a send

- A message being saved locks its attachments (`FOR KEY SHARE`, in
  `message::ensure_attachments_ready`), so the sweep passes them by.
- An app that kept an upload in its composer for longer than a day finds it gone when it sends
  (`attachmentNotReady`), and uploads it again.
