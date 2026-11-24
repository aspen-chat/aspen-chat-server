# Attachment previews

The server makes a preview of each picture, SVG, or video uploaded to a message
(`app::attachment::preview`): a smaller copy of a picture, encoded for the web, an SVG drawn at its
own size the same way, and a poster for a video, one frame of it made the same way. Readers' apps
show the preview inline and the original everywhere else: the gallery, a video's player, and
downloads. An SVG's original is served to be saved, never shown, so its preview is what apps show of
it everywhere.

## Parts

| Part                                                            | Where                                                                                                 |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Decoding, resizing, encoding, running `ffmpeg` and the renderer | `aspen_previews` crate (`server/previews`)                                                            |
| Drawing SVGs, behind a seccomp filter                           | `aspen-svg-render` (`server/svg_render`), a program of its own                                        |
| Reading originals from storage, the queue, keeping what is made | `app::attachment::preview`                                                                            |
| Uploading originals                                             | `app::attachment`, `app::media_store`                                                                 |
| Upload quota                                                    | `app::upload_quota`, table `upload_usage`                                                             |
| Held messages                                                   | `app::message::held`, table `held_message`                                                            |
| Preview queue                                                   | the `makePicturePreview`, `makeSvgPreview`, and `makeVideoPoster` jobs (see [Jobs](../jobs/index.md)) |
| The fonts SVGs' text is drawn in                                | `scripts/svg_fonts.py`, from `client/packages/app/fallbackFonts.json`                                 |

## Pages

- [Uploads](uploads.md): how an original is uploaded, its stored type, the quota, and the limits on
  names and types.
- [What a preview is](preview-format.md): size, encoding, and when a preview is kept.
- [Pictures](pictures.md): decoding pictures in process, and which keep their original.
- [SVGs](svgs.md): SVGs drawn by `aspen-svg-render` behind its limits and seccomp filter, and the
  fonts it draws text in.
- [Videos](videos.md): posters taken by a sandboxed `ffmpeg`.
- [Making previews](making-previews.md): the queue, which servers make them, retries, storage,
  announcing, and deletion.
- [Held messages](held-messages.md): holding a message until its previews are made.
- [Sent before its files arrive](sent-before-uploaded.md): holding a message sent while its files
  are still uploading until each is confirmed, and giving a failed upload another link.
- [Unsent attachments](unsent-attachments.md): sweeping uploads nobody sends.
- [Access](access.md): the revocation checklist.

[Design notes](design-notes.md) hold the reasons behind these choices.
