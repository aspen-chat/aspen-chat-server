# Attachments

| Part                                | Where                                                                                              |
| ----------------------------------- | -------------------------------------------------------------------------------------------------- |
| Uploading                           | `AspenSync.uploadAttachment`, `upload.ts` (`packages/protocol/src/`)                               |
| Inline pictures, videos, and sounds | `src/features/messages/Attachments.tsx` (`MessageMedia`, `InlineVideo`, `InlineAudio`, `keptRoom`) |
| Gallery                             | `ImageGallery.tsx`                                                                                 |
| Records on demand                   | `useAttachment`                                                                                    |
| Describing                          | `AttachmentDescriptionButton`, `AspenSync.describeAttachment`                                      |

## Uploading

Attachments upload in the server's two phases from `AspenSync.uploadAttachment` (`upload.ts`):

1. Reserve. A picture's reservation carries its size, which the composer measures first
   (`measurePicture`). Its record gives it back as `width` and `height`.
2. `PUT` the bytes straight to storage with `uploadFetch`.
3. Confirm. The server moves the bytes from the URL's staging key to where readers fetch them, so
   the URL can change nothing after.

The attachments are then named by id in `sendMessage`.

A message event carries only ids, so `useAttachment` fetches records on demand.

An attachment's name is shown without format and control characters (`plainFileName`), which could
make it read as another name.

## Inline pictures

Images render inline (`src/features/messages/Attachments.tsx`):

- image attachments;
- the pictures of link previews, from links whose path has an image extension or that the server
  found to be images (previews with a picture and no text).

**Every picture is loaded from a deployment's storage, never from the address a message links to.**
Loading the linked address would tell whoever serves it the address of everyone reading. So a link
to a picture shows as a link until its preview arrives with the server's copy, and stays one when
the preview has no picture.

`MessageMedia` gathers the pictures into one strip and shows at most `INLINE_IMAGE_LIMIT` (three)
inline. Beyond that a `+N` tile, like any inline picture, opens `ImageGallery.tsx`, a modal that
pages through the whole set.

## Previews and posters

Inline, an attachment shows the preview the server made of it where there is one: the record's
`preview`. That is a smaller WebP copy of a picture, fitted within 1920 × 960, or a video's poster.
See the server's [attachment previews](../../../../docs/architecture/attachment-previews/index.md).

- The preview shows at its exact size, so its room is kept as a measured picture's is (`keptRoom`).
- A picture whose preview cannot be loaded falls back to its original.
- One whose original has already loaded keeps it when a preview arrives later
  (`attachmentPreviewed`, which the store applies to the record).
- A record read from before the preview was made keeps the preview the store already holds.
- The gallery shows the original, and its thumbnail strip the previews.

### Videos

- A video with a poster shows the poster with a play control (`InlineVideo`). Pressing it swaps in a
  `<video>` of the original in the poster's room, so nothing of the video is fetched until the
  reader asks.
- A video the browser cannot play, and one without a poster, is a download chip.

### Sounds

An `audio/` attachment plays in a small player (`InlineAudio`): its name, which downloads it, a mute
control and a volume, a play and pause control, a position to scrub, shaded up to where it has
played (`SliderRail`'s `filled`), and the time played against its length (`formatClock`).

- It is one size whatever it plays, so it keeps its room from the first.
- Only the file's start is fetched (`preload="metadata"`), for its length, until it is played.
- Dragging the position moves the sound when the thumb is let go. Arrow keys move it by a second, or
  a tenth of one in a sound under ten seconds (`scrubStep`).
- A sound whose file does not give its length (a recording streamed as it was made) plays with the
  position disabled until the browser finds its end.
- It plays through the system's default output, as a video does.
- Each player starts at the volume the reader last chose in any of them (`chosenVolume`), for as
  long as the page lasts. Turned all the way down, it shows as muted, and unmuting it brings it back
  to full.
- iOS keeps every element at full volume, leaving it to the device's buttons, so there the player
  has a mute control alone (`volumeSettable`).
- A sound the browser cannot play is a download chip. The server serves the audio types browsers
  play as themselves (`app::attachment::INLINE_TYPES`) and any other as `application/octet-stream`,
  which no browser plays.

## Descriptions

A picture or video may carry a description, in its uploader's words, for readers who cannot see it.

- It is the attachment record's `description`. Blank is none.
- At most `ATTACHMENT_DESCRIPTION_MAX_CHARS` (1500 characters), the server's
  `app::attachment::DESCRIPTION_MAX_CHARS`.

### Where it shows

| Place                              | How                                                                                 |
| ---------------------------------- | ----------------------------------------------------------------------------------- |
| Inline and in the gallery          | The picture's text alternative (`pictureAlt`, which falls back to "Image: {name}"). |
| Beneath the picture in the gallery | Shown, and hidden from assistive technology there, which has it already.            |
| A video's download chip            | Its `aria-description`.                                                             |

### Writing one

In the message box each picture or video waiting to be sent has a describe control
(`AttachmentDescriptionButton`, filled once described). Its modal keeps the text in the composer's
`Pending`. The server takes it with `PATCH /attachments/{attachment}`
(`AspenSync.describeAttachment`):

- at once, when the file is uploaded;
- when the upload ends, when it is not;
- again before sending, for any that did not arrive.

So the message always goes with what was written.

### Who may write and read it

- The server accepts a description from the uploader alone, at the reservation or by that `PATCH`.
- It accepts one only until the attachment is in a message. A sent description is part of the
  message as it was sent.
- Whoever may read the attachment reads its description. It is seen, and stops being seen, exactly
  as the attachment is (`app::attachment`).
- A plugin shown the message is shown it in the attachment's record (`spec/plugin.wit`).
