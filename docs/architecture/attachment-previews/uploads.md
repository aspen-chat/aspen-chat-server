# Uploads

An original is uploaded as `app::attachment` and `app::media_store` describe. The client uploads straight to storage on a signed URL, then confirms the upload.

## Steps

1. The client declares the file's type and size. `byteSize` is required, and at most `[media] max_attachment_bytes`.
2. Starting the upload records its bytes against the uploader's [quota](#upload-quota).
3. The upload URL is signed for exactly that size and for one `Content-Type` (`contentType` on the handle; see [Stored type](#stored-type)).
4. Confirming refuses, deleting it, an upload over the limit.
5. Confirming moves the upload into place with that type and a `Content-Disposition`: `inline` or `attachment`, with the file's name. SeaweedFS's read path ignores it and gives its own.
6. The copy names only the object it weighed, by its `ETag` (`x-amz-copy-source-if-match`). So a second upload to the same URL between the weighing and the copy is weighed afresh rather than copied unweighed.
7. Confirming a picture or video queues its preview (see [Making previews](making-previews.md)).

## Stored type

The `Content-Type` the URL is signed for is:

- the declared type's essence, when it is among `app::attachment::INLINE_TYPES` (pictures, video, and sound browsers play, plain text, PDF). Its parameters are dropped, but for a plain text's `charset`.
- otherwise `application/octet-stream`.

**Why:** the anonymous-read path serves objects with the type they were stored with, and a browser opening an HTML page, an SVG, or XML from it would run what it holds.

The record keeps the declared type. Apps show it, and it decides whether a preview is made.

## Limits

| Limit | Value | Where |
| --- | --- | --- |
| Attachment size | `[media] max_attachment_bytes` | Signed into the URL, checked on confirming |
| File name | 255 characters | `app::attachment::MAX_FILE_NAME_CHARS` |
| Declared type | 255 bytes of printable ASCII with no commas or quotes | `MAX_MIME_TYPE_BYTES`, `declarable` |
| Attachments per message | 50 | `app::message::MAX_ATTACHMENTS` |

- The type rule means nothing the uploader writes after the essence, such as `text/plain;x=,text/html`, can reach the header a browser reads as several types.
- The attachment count is checked with the rest in `ensure_attachments_ready`, as a message is sent, held, or edited (`messageTooManyAttachments`). The message box uploads no more than 50.

## Upload quota

Each person may upload at most the deployment setting `upload_quota_gib` (25 GiB; 0 sets no limit) in any 24 hours, attachments and icons together (`app::upload_quota`).

- Starting an upload records the bytes its URL is signed for in `upload_usage`, under a lock on the uploader's row.
- The record stands whatever then becomes of the upload (confirmed, sent, deleted, or swept). So deleting files does not make room.
- An upload that would pass the quota is refused with `429` `uploadQuotaExceeded`.
  - Its `detail` names the quota and how long until there is room for the file.
  - Its `Retry-After` says the same in seconds. There is none when the file alone is larger than the quota.
- Records older than a day are pruned with the upload sweep.
