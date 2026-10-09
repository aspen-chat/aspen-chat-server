# Attachment previews

The server makes a preview of each picture or video uploaded to a message (`app::attachment::preview`): a smaller copy of a picture, encoded for the web, and a poster for a video, one frame of it made the same way. Readers' apps show the preview inline and the original everywhere else: the gallery, a video's player, and downloads.

## Parts

| Part | Where |
| --- | --- |
| Decoding, resizing, encoding, running `ffmpeg` | `aspen_previews` crate (`server/previews`) |
| Reading originals from storage, the queue, keeping what is made | `app::attachment::preview` |
| Uploading originals | `app::attachment`, `app::media_store` |
| Upload quota | `app::upload_quota`, table `upload_usage` |
| Held messages | `app::message::held`, table `held_message` |
| Preview queue | the `makePicturePreview` and `makeVideoPoster` jobs (see [Jobs](../jobs/index.md)) |

## Pages

- [Uploads](uploads.md): how an original is uploaded, its stored type, the quota, and the limits on names and types.
- [What a preview is](preview-format.md): size, encoding, and when a preview is kept.
- [Pictures](pictures.md): decoding pictures in process, and which keep their original.
- [Videos](videos.md): posters taken by a sandboxed `ffmpeg`.
- [Making previews](making-previews.md): the queue, which servers make them, retries, storage, announcing, and deletion.
- [Held messages](held-messages.md): holding a message until its previews are made.
- [Unsent attachments](unsent-attachments.md): sweeping uploads nobody sends.
- [Access](access.md): the revocation checklist.

[Design notes](design-notes.md) hold the reasons behind these choices.
