# Sending while files upload

A message can be sent before its files have finished uploading. It goes to the server at once, which
holds it until each file has arrived and posts it then (the server's
`docs/architecture/attachment-previews/sent-before-uploaded.md`). Meanwhile the message stands at
the foot of its channel's list, faded, and the uploads go on whatever the reader does next. On a
phone they go on with the app in the background. [Why](sending-while-uploading-design-notes.md)

## Where it lives

| Part                                    | Code                                                                       |
| --------------------------------------- | -------------------------------------------------------------------------- |
| The session's uploads                   | `AttachmentUploads` (`@aspen/protocol`, `uploads.ts`), `AspenSync.uploads` |
| A browser's transport                   | `browserUploadTransport` (`uploads.ts`)                                    |
| A phone's transport                     | `src/api/uploadsBridge.ts`, the `AspenUploads` plugin in each mobile app   |
| Sending, and taking a message back      | `Composer` (`attachmentsToSend`, `handOver`, `takeBack`)                   |
| The unsent messages in the list         | `PendingMessages.tsx`                                                      |
| Asking before the page goes             | `src/api/sendingGuard.ts`, the desktop shell's `sendingGuard.ts`           |
| Saying a failure wherever the reader is | `src/api/uploadFailures.ts`                                                |

## The session's uploads

An upload belongs to the session, not to the message box that began it. `AttachmentUploads` holds
every upload under way or failed:

1. `add(file)` reserves an attachment (`POST /attachments`) and hands the bytes to the transport,
   which sends them to storage and confirms the upload.
2. Once reserved, the upload knows its attachment's id (`reserved`), which a message can name before
   the bytes have arrived.
3. Once confirmed, its record is the store's, `settled` answers it, and the upload leaves. So the
   store having an attachment's record means it has arrived, and an upload still listed is on its
   way or failed.
4. One that fails stays, with why (`cause`: `refused` in the server's words, `unreachable`, or
   `interrupted`), until it is sent again (`retry`) or discarded.

- **Sending again** asks the server for another link for the same attachment
  (`POST /attachments/{attachment}/renew`), so a message already naming it needs no change. One
  whose reservation itself failed is reserved anew.
- **Changes** to what a file shows, or whether it is a spoiler, reach the server as soon as the
  attachment is reserved (`change`), not when the upload ends, so a message sent meanwhile carries
  them.
- **Discarding** stops the transport and deletes the reservation.

A message box watches the uploads it began (`Composer`'s `follow`). One that fails in a box is
discarded and its file shown failed, as before. Uploads still in a box when the box goes (another
channel opened) are discarded.

## Sending

Send is offered while files upload. `Composer.send` then:

1. Waits for each file's reservation (`attachmentsToSend`), which is a request already made when the
   file was chosen, and rarely still open.
2. Sends as it always does, naming every attachment, ready or not, with `awaitUploads`:
   `sendMessage`, `replyInThread` for a thread's first reply, `invokeCommand`, or `scheduleMessage`
   while a time is held over the box.
3. Hands the uploads over to the session (`AttachmentUploads.detach`) and empties the box.

The server answers `202` with the held message, `waiting: "uploads"`, which the store keeps as it
keeps every held message. A message whose last file arrived as it was sent may be posted at once
instead.

## In the list

`PendingMessages` stands under the newest message of a channel's list, when the list is at the
present. It draws what the caller sent there that the server holds for its files:

- held messages and commands for which `heldInPlace` holds: `waiting` is `uploads`, or `preview` for
  one that waited for its uploads (`awaitedUploads`), so a message does not move from the list to
  above the box for the moments its preview takes;
- scheduled messages whose `status` is `uploading`.

They are left out above the box (`HeldMessages`, `ScheduledStrip`), where everything else the server
holds waits. A message that goes on to wait for its channel's mana moves there, as does one the
server dropped.

Each is drawn as the message it will be: the caller's picture and name, its text as Markdown (a
command's as the server words it), and its files.

- **Faded** to 80% (`.not-yet-sent`). At more contrast, or in forced colours, it is at full
  strength; the words beside the name say it is not sent.
- **Beside the name**, what it waits for: its files, its preview, or, scheduled, its files before
  its time.
- **Each file** is one of:
  - on its way from this device, with a bar for how far, and its picture from the bytes held here;
  - failed here;
  - arrived, with its name and picture from its record;
  - on its way from another device, which only that device can say more about.
- **A failure** is said once under the message, in the danger colour: the first that can be put
  right, else the first. It is said in a toast as well (`tellUploadFailures`), since the reader may
  have left the channel since sending.

| Button    | Shown when                                               | Does                                                                   |
| --------- | -------------------------------------------------------- | ---------------------------------------------------------------------- |
| Try again | An upload of it failed and its bytes are still held      | `AttachmentUploads.retry` for each                                     |
| Edit      | It is a message, its box is open, and every file is here | Takes it back into the box (`takeBackIn`)                              |
| Delete    | Always                                                   | Cancels it with the server and discards its uploads still on their way |

Messages sent after it are posted ahead of it, and it stays under them until its own files arrive.

### Taking one back

Edit cancels the message with the server (`cancelHeld`, or `unschedule` for a scheduled one) and
makes it what the box holds (`Composer.takeBack`): its text and tags read back from what was sent,
what it replied to, a scheduled one's time, and its files. Files that have arrived come back as
records. Files still on their way become the box's uploads again (`AttachmentUploads.attach`), with
no byte sent twice. It is refused while the box holds something else.

A command is not taken back: what the server holds is its wording of the command, not what was
typed.

### On another device

The author's other devices hear of the message (`heldMessageChanged`) and show it the same way, its
files as on their way from another device, with Delete alone. Cancelled from there, the device
sending its files stops them (`AspenSync`, on `heldMessageCancelled`).

## When the page goes

A browser's page sends the bytes itself, so they stop when it goes.

- **Before it goes**, the tab asks, in the browser's own words, while a sent message still waits for
  an upload (`sendingGuard.ts`, `beforeunload`). The desktop app's window asks in the reader's
  language instead: the page tells the shell while that is so (`aspenDesktop.sending`), and the
  shell asks as the window closes (`packages/desktop/src/main/sendingGuard.ts`).
- **Afterwards**, the next page finds the message still held and no upload for it. Uploads handed
  over to the session are noted in `localStorage` for that (`AttachmentUploads`, `restore`), per
  account: the message says the app closed before its file arrived, and is deleted from there, or
  taken back for its text.
- The server lets a message wait a day for its files, then drops it and says so.

At each start, uploads whose message the server no longer holds are let go (`retain`).

## On a phone

A phone's page stops when the app goes to the background, so there the app's native code sends every
upload (`nativeUploadTransport`), from the moment a file is chosen:

1. The page hands the native side the file's bytes, a megabyte at a time (`begin`, `write`), which
   it keeps in the app's own storage.
2. `send` gives it where they go, and the session to confirm with: the deployment, the account, and
   its session and refresh tokens.
3. The native side sends the bytes whether or not the app is in front, then confirms the upload with
   the deployment itself (`POST /attachments/{id}/confirm`), refreshing the session if it has
   expired and asking for another link if storage says the first has
   (`POST /attachments/{id}/renew`). That confirmation is what has the server post the message, so
   it is posted the moment the last file arrives, with no page awake.
4. The page hears how far each has come and how it ended (`progress`, `settled`), and asks again
   each time the app comes back to the front, since what ended while it slept may not have been
   heard.

The native side keeps what it is sending across the page's reloads and the app's restarts. A page
that starts finds them (`kept`) and follows each to its end, or sends it again from the bytes kept
there.

An upload that fails while the app is not in front is told by a notification, in the words the page
gave when it began.

| Platform | How it keeps sending                                                                         |
| -------- | -------------------------------------------------------------------------------------------- |
| iOS      | A background `URLSession`, which the system runs outside the app and wakes the app to finish |
| Android  | A foreground service of type `dataSync`, with a notification standing while files are sent   |

### iOS

`UploadCenter` (`ios/App/App/UploadCenter.swift`) is made at launch, before any page, since the
system may start the app only to tell it how a transfer ended. `AspenUploadsPlugin` passes the
page's calls to it.

- **The transfer** is an upload task of the background session `com.aspen-chat.client.uploads`, from
  the spool file (`Application Support/uploads/<id>`, kept out of backups, readable while the phone
  is locked).
- **When it ends**, the system wakes the app (`handleEventsForBackgroundURLSession` in
  `AppDelegate`), and the center confirms the upload inside a background task of its own. It tells
  the system its events are handled only once every confirmation they began has ended.
- **A confirmation the deployment does not know** (`404`) may be one it already took, whose answer
  never arrived. The center reads the attachment, which answers only once confirmed, to tell.
- **Its records**, sessions among them, are one keychain item of the app's own access group,
  readable after the phone's first unlock and never backed up (`UploadStore`). A record's tokens and
  link are emptied once it has ended. Records two days old go, with their files.
- **An app closed from the switcher** has its transfers cancelled by the system; they are sent again
  from their spool files when it is next opened.
- **A failure with the app not in front** is a local notification, where notifications are already
  allowed.

Nothing is asked of the user, and no background mode or entitlement is declared.

### Android

The uploads are the process's (`DeviceUploads`, one `Uploads` for the app), not the page's or a
service's. `AspenUploadsPlugin` passes the page's calls to them, and `UploadService` keeps the
process alive and its processor awake while there is something to send.

- **The transfer** is a streamed `PUT` of the spool file (`files/uploads/<id>`), at most three at a
  time, each on a thread of its own (`UploadSender`). A write that makes no progress for a minute is
  dropped and tried again.
- **The service** is a foreground service of type `dataSync`, started when an upload is sent and
  stopped when none is left, with an ongoing notification in the words the page gave. It is sticky:
  started again after the process is killed, it sends again, from their spool files, whatever the
  records say was still being sent. Where the system will not let it enter the foreground, the
  uploads go on for as long as the process is left alone, and again when the app is next opened.
- **Android 15's allowance** of six hours a day for such a service ends it (`onTimeout`): uploads
  still under way then fail as unreachable, to be tried again.
- **Its records**, sessions among them, are in the app's private preferences (`aspen_uploads`), as
  the notification code's are. Records two days old go, with their files.
- **A confirmation the deployment does not know** is told from one it already took as on iOS.
- **A failure with the app not in front** is a notification on a channel of its own, where
  notifications are allowed; opening the app clears it.

The manifest declares `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, and `WAKE_LOCK`. Nothing
is asked of the user.

### Both

- A deployment that does not answer, or answers that it cannot now (`408`, `5xx`, a `429` that is
  its rate limit), is asked again a few times with growing waits before the upload fails as
  unreachable. Any other refusal fails it with the deployment's words, asked for in the app's
  language (`acceptLanguage`).
- Storage answering `403` is taken for an expired link: another is asked for, twice at most.
- `android/app/src/test/.../uploads/UploadsTest.java` runs Android's upload logic on the JVM against
  a stand-in for storage and the deployment. iOS's has no test in the tree.

## Tests

- `packages/protocol/test/uploads.test.ts`: the uploads themselves, against a transport the test
  holds.
- `e2e/sendWhileUploading.spec.ts`: sending, the unsent message and its audit, taking it back,
  deleting it, a failed upload sent again, and a message another device is uploading.
- `e2e/attachmentDescriptions.spec.ts`: a description given while a picture uploads reaches the
  server before the picture has.
- The server's `scripts/check_permissions.py`, `speculative_sends`.
