# Attachment previews: design notes

The reasons behind the [attachment previews](index.md) design.

## Why previews at all

- **The server makes a smaller copy of every picture and a poster of every video.** A picture is
  shown inline at no more than 320 CSS pixels tall, while a phone's camera writes several megabytes
  of pixels it will never be shown at.

## Uploads

- **Types outside `INLINE_TYPES` are stored as `application/octet-stream`.** The anonymous-read path
  serves objects with their stored type, and a browser opening HTML, SVG, or XML from it would run
  what it holds. See [Uploads](uploads.md#stored-type).
- **The copy is conditional on the weighed `ETag`.** A second upload to the same URL between
  weighing and copying would otherwise be copied unweighed.
- **Declared types are printable ASCII with no commas or quotes.** Nothing written after the essence
  can reach the header a browser reads as several types.

## Sent before its files arrive

- **The server holds the message, not the app.** A phone's page stops when the app goes to the
  background, and a page that is not running cannot post. With the message already at the server,
  all a device must still do is send bytes and confirm them, which a phone's native code does with
  no page awake, and the posting happens in the request that confirms the last file. The author's
  other devices see it waiting, and it is checked where every message is checked.
- **The confirmation sets it going, in its own transaction.** A job that looked for confirmed files
  on a timer would post the message a look late. The job remains for the two things no request does:
  giving up after a day, and a file deleted.
- **Each attachment is held against being confirmed while the message is saved.** Without that, a
  confirmation between the check and the insert would see no message to set going, and the message
  would wait a day for a file already there. See
  [A file confirmed as it is sent](sent-before-uploaded.md#a-file-confirmed-as-it-is-sent).
- **A day's wait**, as long as a confirmed upload waits to be sent. A large file on a slow link may
  take hours; nothing tells the server an upload has died short of its never arriving.
- **Fifty at once.** One confirmation sets going every message waiting for that file, so their
  number is bounded for the request's sake; fifty is more than anyone sends before the first
  arrives.
- **A failed upload keeps its attachment.** Another reservation would mean changing the message that
  already names the first, on a phone possibly with no page to do it. Another URL for the same
  attachment changes nothing else.
- **It does not keep later messages waiting.** An upload may take as long as it takes; what its
  author says meanwhile should not.

## The preview box

- **960 pixels tall** is the 320 CSS pixels a picture may stand inline, on a screen of three device
  pixels to the CSS pixel. See [What a preview is](preview-format.md).
- **1920 pixels wide** is the widest message column at two device pixels to the CSS pixel.
- **Quality 85 with sharp YUV.** At that density it shows no loss, and sharp YUV keeps fine
  saturated detail (red text on white) from smearing.
- **A preview less than ten percent smaller is not kept.** An original already small and web-ready
  is shown as it is, and storage is not spent on a copy no smaller.

## Pictures

- **Pictures are decoded in process, by decoders written in Rust, with an allocation limit.** The
  bytes are anyone's. See [Pictures](pictures.md).
- **Moving pictures keep their original.** A preview would hold the first frame only.
- **Pictures are brought into sRGB from their profile.** A phone's Display P3 photo keeps its
  colours. Ones that cannot be brought over faithfully keep their original.

## Videos

- **Video is not transcoded.** A minute of video costs minutes of every core a server has to encode,
  competing with everything else it serves. A frame takes about a second however long the video. See
  [Videos](videos.md).
- **Apps fetch nothing of a video until asked.** The poster stands in, with a play control.
- **`ffmpeg` and `ffprobe` run as processes of their own, on a copy in a scratch directory.** A file
  that crashes a decoder, or takes more memory than it should, takes nothing else with it.
- **Protocol and format whitelists.** A file made to look like a playlist cannot have them fetch
  addresses or read other files.
- **HDR video has no poster.** Its frame flattened to SDR without tone mapping would look washed
  out.

## Making previews

- **Previews are jobs, made only by servers that can make them.** Only servers with `make` and
  `[jobs] run` on claim them, and only those that can run `ffmpeg` claim posters. See
  [Making previews](making-previews.md).
- **A preview job unmade for seven days is deleted.** One may wait that long when no server can run
  `ffmpeg`.
- **Previews are stored under `attachment-previews/`, not under the original's key.** Some object
  stores cannot hold a key as both an object and a prefix.
- **Recording and announcing lock the attachment row first.** A message taking up the attachment at
  the same moment is either seen by the announcement or written after it commits.
- **A preview is never taken away once made.** A client can keep one it heard of over an older
  record.

## Held messages

- **Messages may be held until their previews are made.** Otherwise a message would reach its
  readers showing the original inline, then switch to the preview as it landed. See
  [Held messages](held-messages.md).
- **Holding needs `mayHold`.** Older clients and other deployments' do not show their sender a
  waiting message, so their messages post at once.
- **The held row is deleted in the posting's transaction.** It is posted once however its job is
  run.
- **An author's held messages wait for earlier ones still held.** So they are posted in the order
  they were sent.
- **Posting rechecks the author's permissions and the channel's plugins.** It decides the message as
  things stand when it is posted.
- **A sign-out or password change does not drop a held message.** It was sent while signed in, as a
  message posted at once would have been.

## Unsent attachments

- **Unsent uploads are swept after a day.** The store is not a file host for uploads nobody sends.
  See [Unsent attachments](unsent-attachments.md).
