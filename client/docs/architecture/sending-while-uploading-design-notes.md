# Sending while files upload: design notes

The reasons behind [sending while files upload](sending-while-uploading.md).

## The message goes to the server at once

- **The app does not wait for the uploads and then send.** A phone's page runs no code in the
  background, so a message kept in the page would not be sent until the app was opened again. With
  the message already at the server, the only work left is sending bytes and confirming them, which
  native code can do alone, and the server posts in the request that confirms the last file.
- **Other devices see it.** It is the server's record, told to every device of its author, rather
  than something one page remembers.

## Uploads belong to the session

- **Not to the message box.** A box goes when its channel is left, and a sent message's uploads must
  outlive it. One place also answers what the list asks of every unsent message: which of its files
  are still on their way from here.
- **An upload that arrives leaves.** The store's record then says all there is, and its absence
  tells an upload still on its way, which is why a change to an unconfirmed attachment is not kept
  in the store.

## On a phone, every upload is native from the start

- **Not only once the message is sent, or once the app is backgrounded.** An upload to a presigned
  link cannot be resumed, so handing one from the page to native code part way means sending the
  file again from its first byte. Native from the moment the file is chosen, nothing changes hands.
- **The bytes cross the bridge once**, in pieces, as received files do (`AspenFiles`). That is one
  path for a file however it was chosen: the picker, a paste, text too long for a message.
- **The native side confirms.** In the background no page can, and one owner of the confirmation is
  simpler than two that might both try.
- **It holds a session.** Confirming is the signed-in user's request. The tokens are kept as the
  notification code's are, and refreshed the same way.

## In the list, faded

- **At the foot of the list**, where it will be, rather than above the box with what the channel
  holds up: the author has sent it, and reads on.
- **Through the preview's hold too.** Its files arrived, it would otherwise jump to above the box
  for the moments a preview takes, then into the list.
- **80%, and full strength at more contrast.** Faded says not yet; a reader who asked for contrast
  gets the words instead.

## What it does not do

- **It does not keep later messages waiting.** An upload takes as long as it takes.
- **A command is not taken back into the box.** The server holds its wording, with ids where names
  were typed.
- **A page's uploads do not survive the page.** The browser gives a page nothing that would; the tab
  asks before it goes, and says afterwards what was lost.
