# Unsent attachments

An attachment is meant to be sent. One nobody sends is deleted, so the store is not a file host for uploads nobody sends.

## The sweep

`attachment::sweep_unsent` deletes an attachment's row, object, and preview when:

- it was confirmed a day ago (`app::attachment::UNSENT_LIFETIME`);
- it has never been in a message; and
- it is not held in one.

It runs as part of the recurring `sweepUploads` job (`media_store::sweep_step`, hourly; see [Jobs](../jobs/kinds.md#sweepuploads)), 500 at a time under `FOR UPDATE SKIP LOCKED`.

## Sent attachments

- `attachment.sent` records that an attachment has been in a message. It is set as the attachment is put in one.
- One an edit has since taken out of its message is not swept.

## Racing a send

- A message being saved locks its attachments (`FOR KEY SHARE`, in `message::ensure_attachments_ready`), so the sweep passes them by.
- An app that kept an upload in its composer for longer than a day finds it gone when it sends (`attachmentNotReady`), and uploads it again.
