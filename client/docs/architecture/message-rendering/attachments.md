# Attachments

| Part                                | Where                                                                                              |
| ----------------------------------- | -------------------------------------------------------------------------------------------------- |
| Uploading                           | `AspenSync.uploadAttachment`, `upload.ts` (`packages/protocol/src/`)                               |
| Inline pictures, videos, and sounds | `src/features/messages/Attachments.tsx` (`MessageMedia`, `InlineVideo`, `InlineAudio`, `keptRoom`) |
| Text and code files                 | `TextAttachment.tsx`, `textPreview.ts`                                                             |
| Spoilers                            | `SpoilerCover` in `SpoilerCard.tsx`, `SpoilerToggle` in `Composer.tsx`                             |
| Gallery                             | `ImageGallery.tsx`, `gallerySwipe.ts`, `useGalleryZoom.ts`, `galleryZoom.ts`                       |
| Records on demand                   | `useAttachment`                                                                                    |
| Describing                          | `AttachmentDescriptionButton`, `AspenSync.updateAttachment`                                        |

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

- image attachments the deployment serves for showing in place, and those it serves to be saved
  (`Attachment.inline` false, an SVG among them) once they have a preview;
- the pictures of link previews, from links whose path has an image extension or that the server
  found to be images (previews with a picture and no text).

**Every picture is loaded from a deployment's storage, never from the address a message links to.**
Loading the linked address would tell whoever serves it the address of everyone reading. So a link
to a picture shows as a link until its preview arrives with the server's copy, and stays one when
the preview has no picture.

Which previews are pictures and which are cards is sorted by `linkPictures` (`linkPictures.ts`). It
matches a link in the text to its preview by address, each as a WHATWG URL serializes it
(`normalizedUrl`), since the server writes a preview's `url` that way and the text holds the link as
it was typed.

`MessageMedia` gathers the pictures into one strip and shows at most `INLINE_IMAGE_LIMIT` (three)
inline. Beyond that a `+N` tile, like any inline picture, opens `ImageGallery.tsx`, a modal that
pages through the whole set with arrows, the arrow keys, and a strip of thumbnails. On a touch
screen (`TOUCH_SCREEN`, any coarse pointer, a touch laptop's included) a finger swipes between the
pictures too (`useSwipe`, the hook the drawers follow a finger with): the current picture and its
neighbours are drawn side by side and follow the finger, and when it lifts they go the way it was
thrown or to whichever picture is more than halfway in (`swipeOutcome` in `gallerySwipe.ts`), easing
into place unless motion is reduced; pulled past the first or last picture they give a little
(`rubberBand`) and spring back. On a touch-only device (`TOUCH_ONLY`) the arrows are left out, since
they would take width from the picture on a phone.

A finger resting on an inline picture for `LONG_PRESS_MS` opens the picture's own menu
(`PictureMenu.tsx`), a popover beneath it (above it when there is no room beneath): View image,
which opens the gallery on it, Download image, where the clipboard takes a picture, Copy picture,
and last, where the message has a sheet of its actions (a touch-only screen, the message not being
edited), More options, which opens that sheet as the message's own long press does
(`OpenMessageActionsContext`, which `MessageItem` provides). Every picture action takes the original
(`Picture.src`), never the smaller copy shown inline, but Copy picture of one whose original is
served to be saved, which copies what is shown of it (`Picture.shown`, `shownSrc`). The timer is the
picture's own, on the button's press events: the button's press goes no further, so the message's
long press never sees it, and letting go once the menu has opened is not a press on the picture.
Android answers a finger's long press with `contextmenu`, which the picture keeps from the message's
menu while a finger presses it; a pointer's right click goes on up to the message's context menu,
which offers the same actions (see [context menus](../context-menus.md)).

### Zooming

On a touch screen fingers zoom the gallery's current picture (`useGalleryZoom`, whose arithmetic is
`galleryZoom.ts`):

- Two fingers pinch it about their middle, and pan it as they move together.
- A double-tap zooms in on the point tapped, to `DOUBLE_TAP_ZOOM` times its fitted size. A
  double-tap on a zoomed picture fits it again.
- One finger pans a zoomed picture, and thrown, the picture coasts. On a fitted picture one finger
  swipes between pictures, as above.
- A pan that reaches the picture's side carries on into the next picture, as a swipe would. The
  zoomed picture slides away as it is, and is fitted again once out of sight.
- A picture zooms to `ZOOM_OVER_FIT` (four) times its fitted size, or further where its original has
  more to show: to one of its pixels per CSS pixel (`maxScale`).
- Zoomed past a limit, or panned past an edge, it gives a little and eases back when the fingers
  lift. Where motion is reduced it follows the fingers, settles at once, and does not coast.
- A zoomed picture spreads over the whole gallery, under the counter and the controls beside it. Its
  description and the thumbnails are put away until it is fitted again.
- It pans across the stage, the room a fitted picture has, and never out of it (`panRange`).
- Moving to another picture fits it again. A covered spoiler does not zoom.
- The pictures stand the gallery's whole width apart, so a neighbour waits outside the gallery
  however far a zoomed picture spreads.
- The browser's own pinch and double-tap zoom are off over the gallery (`touch-action`).

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

### Pictures served to be saved

The record's `inline` says whether `downloadUrl` serves the file for showing in place. It is false
for an SVG, which storage serves as `application/octet-stream` because a browser opening it would
run its scripts, and for every other type outside the server's `INLINE_TYPES`; a deployment that
predates the field leaves it out, and is taken to serve every picture's original to be shown
(`servedInline`).

- Such a picture with a preview is shown by the preview alone, inline and in the gallery
  (`Picture.shown`), and at the preview's size; its original is never loaded as a picture, since it
  would not draw. Saving it saves the original, and copying it copies the preview.
- One whose preview cannot be loaded is unavailable, with no original to fall back to.
- One without a preview yet is not a picture: an SVG shows as text (`TextAttachment`, which takes
  `.svg`), any other as a download chip, until `attachmentPreviewed` brings its preview.
- `pictureSrc` (`images.ts`) is where every other view takes a picture attachment's address from:
  thumbnails while composing and editing, pending messages' chips, and the height a message is
  estimated at (`useMessageGroups`).

### Videos

- A video with a poster shows the poster with a play control (`InlineVideo`). Pressing it swaps in a
  `<video>` of the original in the poster's room, so nothing of the video is fetched until the
  reader asks.
- A video the browser cannot play, and one without a poster, is a download chip.

### Sounds

An `audio/` attachment plays in a small player (`InlineAudio`): its name, which downloads it, a mute
control and a volume, a play and pause control, a position to scrub, shaded up to where it has
played (`SliderRail`'s `filled`), and the time played against its length (`formatClock`).

- It is as wide as its whole name needs, from 24rem up to the width the message has, where the name
  is cut short; otherwise it is one size whatever it plays, so it keeps its room from the first.
- Once it has played to its end, the play control replays it, shown as an arrow turning back against
  the direction of reading (counter-clockwise, clockwise right to left), and the position sits at
  the end it reached.
- Only the file's start is fetched (`preload="metadata"`), for its length, until it is played.
- Dragging the position moves the sound when the thumb is let go. Arrow keys move it by about a
  second, or a tenth of one in a sound under ten seconds, a step that divides its length exactly
  (`scrubSteps`): the slider counts those steps rather than seconds, since React Aria snaps a value
  to a whole number of steps and would hold the thumb short of a length that is not one.
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

## Text and code files

A text or code file shows its first `TEXT_PREVIEW_LINES` (20) lines in its message, as a code block
under its download chip (`TextAttachment`).

- Which files: those whose declared type is text (`text/*`, JSON, XML, scripts; `TEXT_TYPES`), and
  those whose extension or whole name is a text or source file's whatever their type, since most
  systems declare none for source files (`textLanguage`, `textPreview.ts`).
- Code is highlighted by its extension, which highlight.js resolves through its aliases (`rs`,
  `py`); prose and data (`.txt`, `.log`, `.csv`) show as written. See [Markdown](markdown.md) for
  the highlighter and its length limit.
- The lines are read from storage as a picture is, without credentials, and reading stops at the
  line past the twentieth or at `TEXT_PREVIEW_MAX_BYTES` (32 KiB), whichever comes first, cancelling
  the rest of the download. What was read is kept for the `KEPT_HEADS` (64) files asked for most
  recently, and one read serves every message showing the file at once.
- A file with more than it shows ends in a Download to see more button, centred, in the app's own
  type, which saves the whole file (`saveFrom`).
- Until its lines arrive a few lines' worth of skeleton stands in; the list keeps the view still
  when they replace it.
- A file that cannot be read (storage that sends no `Access-Control-Allow-Origin`), or that holds a
  NUL and so is not text, shows as its download chip alone.

## Spoilers

An attachment's uploader may mark it a spoiler before sending it, and readers' apps cover it until
each reader reveals it.

- It is the attachment record's `spoiler`, false when absent (as from a deployment that predates
  it). The server takes it at the reservation or by `PATCH /attachments/{attachment}`, from the
  uploader alone and only until the attachment is in a message, as a description
  (`app::attachment::update_attachment`).
- In the message box each file waiting to be sent has a toggle (`SpoilerToggle`, an eye struck
  through, filled while on), and a picture's thumbnail blurs while it is on. The mark reaches the
  server as a description does: at once, when the upload ends, and again before sending
  (`AspenSync.updateAttachment`).
- Every kind is covered (`SpoilerCover`): a picture, a video's poster, a sound's player, a text or
  code file's card, and a download chip, whose name might give it away. The cover is blurred past
  reading under a Reveal spoiler button, and what it covers is `inert`, as a covered link card is
  (see [link and video cards](link-cards.md#links-inside-spoilers)). It keeps its room, and a text
  file's lines are read all the same, so revealing moves nothing.
- The attachment's own controls (remove, copy ID) stay usable over the cover.
- What a reader has revealed stays revealed while the message is shown (`MessageMedia`'s
  `revealed`), the gallery included. In the gallery a covered picture is covered too, its thumbnail
  blurred, its description left out, and its thumbnail named "Spoiler image".
- A spoiler changes nothing about who may read the attachment, so nothing is added to who observes
  it or when that changes (see [Who may write and read it](#who-may-write-and-read-it)). Its bytes
  are fetched as any attachment's are; the cover is for the reader's eyes, not a secret.
- A plugin shown the message is shown the mark in the attachment's record (`spec/plugin.wit`).

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
(`AspenSync.updateAttachment`):

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
