# Making previews

## Queueing

Confirming an upload whose MIME type is a picture's or a video's queues its preview as a job (see [Jobs](../jobs/index.md)):

- The job is `makePicturePreview` or `makeVideoPoster`, keyed by the attachment.
- It is written in the transaction that confirms the upload (`preview::queue`).
- Its payload holds `holdUntil`: until when messages holding the attachment wait for it (see [Held messages](held-messages.md)).
- What was just uploaded is interactive. What the migration queued for the pictures and videos uploaded before previews were made is bulk.

## Which servers make them

- Only servers whose `[media.previews]` has `make` on, and whose `[jobs]` has `run` on, claim preview jobs.
- Only those that can run `ffmpeg` claim `makeVideoPoster`.
- Each makes at most `concurrency` previews at once. The job runner keeps that bound for the two kinds together, beside its own places (`jobs::bounded`).

## Outcomes

| Outcome | What happens |
| --- | --- |
| Cannot be made now (storage did not answer) | Tried again, waiting a minute and twice as long each time, six times (`preview::MAX_ATTEMPTS`). The messages it holds are let go meanwhile. |
| Cannot be made at all, or not worth keeping | The job is done with. |
| Not made by any server within seven days (`preview::GIVEN_UP_AFTER`) | The job is deleted by `pruneFailedJobs`. One may wait this long when no server can run `ffmpeg`. |
| Kept | Stored, recorded, and announced (below). |

## Keeping a preview

1. Store it at `attachment-previews/{id}`, beside the originals rather than under the original's own key. Some object stores cannot hold a key as both an object and a prefix.
2. In one transaction that first locks the attachment row:
   - record it on the attachment row: `preview_storage_key`, `preview_mime_type`, `preview_width`, `preview_height`, all or none;
   - announce `attachmentPreviewed`, naming the attachment, its message, and the preview.

`attachmentPreviewed` goes to the channel of each message holding the attachment, or to its uploader alone while it is in none.

**Why lock first:** a message taking up the attachment at the same moment is either seen by the announcement, or written after it commits, its readers finding the preview on the attachment.

## The record

The attachment's record carries the preview as `preview`: its URL, MIME type, and size in pixels as it is shown upright. The record is `GET /attachments/{attachment}` and every read that sideloads it.

A preview is never taken away once made, so a client keeps one it heard of over a record read before it was made.

## Deletion

- Deleting an attachment deletes its preview.
- A preview being made as its attachment is deleted is deleted by its job, which finds the row gone.
- A preview whose attachment became evidence for reviewing reports meanwhile (its message deleted, or it taken off its message: `app::attachment::evidence`) is deleted the same way. Its job is deleted as it became evidence, and the messages it held are let go.
- A preview already made moves to `evidence/` with its original.
- Purging a benchmark run purges its previews.

## Metrics

All are labelled by `kind` (`picture` or `video`).

| Metric |
| --- |
| `aspen_attachment_previews_made_total` |
| `aspen_attachment_previews_failed_total` |
| `aspen_attachment_preview_duration_seconds` |
