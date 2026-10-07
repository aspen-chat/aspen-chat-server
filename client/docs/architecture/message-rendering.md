# Message bodies, attachments, and video links

- Message bodies are GitHub-flavoured Markdown, rendered by `src/features/messages/Markdown.tsx`
  with `react-markdown` (no raw HTML, unsafe schemes dropped); element styles are the
  `message-body` rules in `styles.css`. Only an absolute `http:`, `https:`, or `mailto:` address
  becomes a link (`messageLinkUrl`); a relative or protocol-relative one (`//host/share/file`),
  which on a page loaded from a file (the desktop app's) would be a `file:` link, stays plain
  text. A link is always its own address: one its author named with words of their own
  (`[text](url)`, a reference `[text][ref]` and its `[ref]: url` line) and a picture written
  into the text (`![alt](url)`) show as the characters they were written as
  (`remarkLiteralLinks.ts`), so `[my bank](https://evil.example)` cannot pass for a link to the
  bank and a picture is never loaded; the address inside is then linked as any bare address is.
  Autolinks (`<https://…>`) and GFM's literal `https://…` and `www.…` stay links, since their
  text is their address. Only messages are rendered by `Markdown.tsx`. Fenced code is
  highlighted by highlight.js (`src/features/messages/highlighter.ts`), which loads as its own chunk on the first code
  block: its "common" grammars come with that chunk and every other grammar it ships is fetched
  on first use. Token colours are the `code-*` palette tokens, mapped from `hljs-*` classes at
  the end of `styles.css`. Nothing is auto-detected; an unlabelled fence is plain. Spoilers are
  `||text||` (Discord) or `>!text!<` (Reddit), wrapped by `remarkSpoilers.ts` and rendered by
  `Spoiler.tsx` as a block the reader activates to reveal; markup inside them is kept. On top of
  GFM's own autolinks, bare domains are linked by
  the tokenizer in `src/features/messages/linkify.ts`: explicit
  `http(s)` URLs, and bare domains whose TLD is on IANA's list (the `tlds` package). A handful
  of TLDs that double as source-file extensions only link with a port, a path, or `www.`; the
  set is a constant in that file. The server's preview extractor
  (`server/app/src/link_preview/urls.rs`, list in `tlds.txt` beside it) applies the same rule, so what renders
  as a link is what gets a preview; change both together.
- A link to a deployment the user uses (`parseSelfLink` in `src/features/messages/selfLinks.ts`:
  the home, at the address the app reaches it at or the page's own, and every deployment signed
  in to from there) is a chip naming what it leads to rather than its address (`SelfLink.tsx`):
  the name behind each of its ids, outside in, between Phosphor carets, such as
  `Family › general › Message from Bob`, led by the deployment's domain when it is not the one
  the message is from. Every route of the app is read: a deployment's front page, communities,
  channels, messages, threads, DMs, and invites, on its own address or the home's under
  `/at/{domain}` (an invite's `?at=` naming its deployment), and the home's own pages (the
  dashboard and its tabs, adding a bot, registration invites, sign-in codes, attributions);
  the deployment's API, files, and any other path stay ordinary links. Pressing the chip opens
  the route here through the router (`selfLinkRoute`), so the page is not loaded again. Names
  come from the store of the deployment the link is to (`SourceScope`), with a word for the kind
  of thing wherever a name is unknown or not the reader's to see (`unnamed`): a community's and
  channel's from the store, a DM's from its people (read on demand with
  `AspenSync.ensureChannel`, a skeleton meanwhile), a message's from its author as the read of
  the linking message sideloaded it (`include=linked`), a thread as the word "Thread", since a
  thread's starter message names it, and an invite by its code, since reading what an invite
  opens counts against the limit on guessing invites. Its tooltip is its address, and
  right-clicking it, or holding a finger on it (its own timer, which keeps the press from the
  message's actions), opens a menu that copies the address or opens it as an ordinary link
  would.
- Attachments upload in the server's two phases from `AspenSync.uploadAttachment` (`upload.ts`; reserve,
  `PUT` the bytes straight to storage with `uploadFetch`, confirm, which has the server move them
  from the URL's staging key to where readers fetch them, so the URL can change nothing after) and are named by id in
  `sendMessage`. A picture's reservation carries its size, which the composer measures first
  (`measurePicture`), and its record gives it back as `width` and `height`. A message event carries only ids, so `useAttachment` fetches records on
  demand. Images render inline (`src/features/messages/Attachments.tsx`): image attachments,
  and the pictures of link previews, from links whose path has an image extension or that the
  server found to be images (previews with a picture and no text). Every picture is loaded
  from a deployment's storage, never from the address a message links to, which would tell
  whoever serves it the address of everyone reading: a link to a picture shows as a link until
  its preview arrives with the server's copy, and stays one when the preview has no picture.
  `MessageMedia` there gathers them into one strip and shows at most `INLINE_IMAGE_LIMIT` (three) inline; beyond that a `+N` tile,
  like any inline picture, opens `ImageGallery.tsx`, a modal that pages through the whole set.
- Inline, an attachment shows the preview the server made of it where there is one (the record's
  `preview`: a smaller WebP copy of a picture, fitted within 1920 × 960, or a video's poster; see
  the server's `docs/architecture/attachment-previews.md`), at the preview's exact size, so its
  room is kept as a measured picture's is (`keptRoom`). A picture whose preview cannot be loaded
  falls back to its original, and one whose original has already loaded keeps it when a preview
  arrives later (`attachmentPreviewed`, which the store applies to the record; a record read
  from before the preview was made keeps the preview the store already holds). The gallery shows
  the original, and its thumbnail strip the previews. A video with a poster shows the poster with
  a play control (`InlineVideo`), which swaps in a `<video>` of the original in the poster's
  room, so nothing of the video is fetched until the reader asks; a video the browser cannot
  play, and one without a poster, is a download chip.
- A picture or video may carry a description, in its uploader's words, for readers who cannot
  see it: the attachment record's `description` (at most `ATTACHMENT_DESCRIPTION_MAX_CHARS`,
  1500 characters, the server's `app::attachment::DESCRIPTION_MAX_CHARS`; blank is none). It is
  the picture's text alternative inline and in the gallery (`pictureAlt`, which falls back to
  "Image: {name}"), shows beneath the picture in the gallery (hidden from assistive technology
  there, which has it already), and is a video's download chip's `aria-description`. In the
  message box each picture or video waiting to be sent has a describe control
  (`AttachmentDescriptionButton`, filled once described) whose modal keeps the text in the
  composer's `Pending`; the server takes it with `PATCH /attachments/{attachment}`
  (`AspenSync.describeAttachment`) at once when the file is uploaded, when the upload ends when
  it is not, and again before sending for any that did not arrive, so the message always goes
  with what was written. The server accepts a description from the uploader alone, at the
  reservation or by that `PATCH`, and only until the attachment is in a message: a sent
  description is part of the message as it was sent. Whoever may read the attachment reads its
  description, so it is seen, and stops being seen, exactly as the attachment is (`app::attachment`);
  a plugin shown the message is shown it in the attachment's record (`spec/plugin.wit`).
- Video links get a card with a play control (`src/features/messages/VideoCard.tsx`). The
  server only sends a player for providers in its `VIDEO_PROVIDERS` table
  (`server/link_preview/src/video.rs`), and `src/features/messages/video.ts` keeps the matching
  list of player hosts the client will frame; extend both together. Twitch's player needs the
  embedding hostname as `parent`, which `playerSrc` adds. The player iframe is sandboxed and
  only created after the reader presses play.
- Other links get a card (`LinkPreviewCard` in `src/features/messages/MessageBody.tsx`): the
  title, the site name beneath it as a video card has it, two lines of description, and the
  picture beneath them, as wide as the picture or the card, whichever is narrower, and at most
  320px tall as a message's pictures are, with the page's theme colour down its edge. The server writes each field as it
  is shown; a Reddit post's site name is `r/{subreddit} · u/{author}`
  (`server/link_preview/src/reddit.rs`).
- What the deployment's plugins say about a message shows beneath it as chips
  (`MessageAnnotations`), and a message a plugin changed is marked "(changed by …)" beside the
  edited mark (`AlteredBy`); see Plugins.
- Every address a deployment sends, or a message holds, is checked before it becomes a link, a
  picture, or a window (`src/features/layout/safeUrl.ts`), since any deployment the user holds
  a session on writes some of them: pages to open (a link preview's, a plugin card's or
  annotation's) must be `http:` or `https:` (`webPageUrl`), and files a deployment serves
  (attachments, previews, icons) `https:`, or `http:` at a loopback host or while the page is
  itself served over plain HTTP, as a development deployment is (`mediaUrl`). A person's shown email
  address becomes a `mailto:` link of that address alone, everything but its `@` encoded, so it
  cannot add recipients, a subject, or a body (`mailtoUrl`). An attachment at
  any other address is unavailable, and a preview's picture there is left out. The desktop
  shell checks again: the window navigates only to the app's own `index.html` (or the dev
  server), and hands the system only `http:`, `https:`, and `mailto:` links
  (`packages/desktop/src/main/navigation.ts`).
