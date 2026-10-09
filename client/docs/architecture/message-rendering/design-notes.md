# Message rendering: design notes

Rationale behind the [message rendering](index.md) pages.

## Markdown

See [Markdown](markdown.md).

- **Bodies nested past `MAX_NESTING` show as plain text.** Parsing and rendering recurse once per
  level of nesting.
- **Table rows keep only the cells written (`tableRow`).** The default handler pads every row to the
  header's width, so a wide header over many short rows would make millions of cells.
- **Error boundaries wrap the Markdown and each `MessageBody`, and `RouteError` replaces a failed
  route.** One message cannot blank the app.
- **Relative and protocol-relative addresses stay text.** On a page loaded from a file (the desktop
  app's), `//host/share/file` would be a `file:` link.
- **A link is always its own address.** `[my bank](https://evil.example)` cannot pass for a link to
  the bank, and a picture written into the text is never loaded. Autolinks and GFM's literal links
  stay links because their text is their address.
- **The system account's messages render with `links` off.** Its notices quote names others chose (a
  community's, a person's), so an address in such a name must not become a link or fetch a preview.
- **Nothing is auto-detected for highlighting, and long blocks stay plain.** Some grammars take most
  of a second over a few kilobytes written to slow them (`MAX_HIGHLIGHT_LENGTH`).
- **Linkify keeps the runs of recent texts.** A message drawn again is not scanned again.
- **Linkify follows the server's preview extractor's rules.** What renders as a link is what gets a
  preview.

## Links to a deployment

See [links to a deployment](self-links.md).

- **The chip opens the route through the router.** The page is not loaded again.
- **A thread is named "Thread".** A thread's starter message names it.
- **An invite is named by its code.** Reading what an invite opens counts against the limit on
  guessing invites.
- **The long press has its own timer.** It keeps the press from the message's actions.

## Attachments

See [attachments](attachments.md).

- **Confirming moves the bytes from the upload URL's staging key.** The URL can change nothing after.
- **Names are shown without format and control characters.** They could make a name read as another.
- **Pictures load only from a deployment's storage.** Loading the address a message links to would
  tell whoever serves it the address of everyone reading.
- **Previews show at their exact size.** Their room is kept as a measured picture's is.
- **A video's original is fetched only on play.** Nothing of the video is fetched until the reader
  asks.
- **A description is sent at once, when the upload ends, and again before sending.** The message
  always goes with what was written.
- **A description is fixed once its attachment is in a message.** A sent description is part of the
  message as it was sent.
- **The gallery's shown description is hidden from assistive technology.** It has it already as the
  picture's text alternative.

## Cards

See [link and video cards](link-cards.md).

- **The client keeps its own list of player hosts.** It frames only the providers the server's
  `VIDEO_PROVIDERS` table sends players for.

## Address safety

See [address safety](address-safety.md).

- **Every address is checked before it becomes a link, a picture, or a window.** Any deployment the
  user holds a session on writes some of them.

## Message groups

See [message groups](message-groups.md).

- **A group's height is estimated, never measured.** A group is then the same however its pictures
  load.
- **A message keeps its grouping while it stays in the window.** Read afresh, a page of older history
  arriving above the view would move group boundaries all the way down into it.
- **Each grouped message is its own row.** It highlights, takes focus, and opens its actions alone.
