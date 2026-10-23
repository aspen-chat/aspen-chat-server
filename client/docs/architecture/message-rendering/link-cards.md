# Link and video cards

## Video cards

Video links get a card with a play control (`src/features/messages/VideoCard.tsx`).

- The server only sends a player for providers in its `VIDEO_PROVIDERS` table
  (`server/link_preview/src/video.rs`).
- `src/features/messages/video.ts` keeps the matching list of player hosts the client will frame.
  **Extend both together.**
- Twitch's player needs the embedding hostname as `parent`, which `playerSrc` adds.
- The player iframe is sandboxed, and only created after the reader presses play.

## Link preview cards

Other links get a card (`LinkPreviewCard` in `src/features/messages/MessageBody.tsx`). From top to
bottom:

1. the title;
2. the site name beneath it, as a video card has it;
3. two lines of description;
4. the picture, as wide as the picture or the card, whichever is narrower, and at most 320px tall,
   as a message's pictures are.

The page's theme colour runs down the card's edge.

The server writes each field as it is shown. A Reddit post's site name is
`r/{subreddit} · u/{author}` (`server/link_preview/src/reddit.rs`).

A link preview's picture is always the server's copy (see
[attachments](attachments.md#inline-pictures)), and must pass the address checks in
[address safety](address-safety.md).

## Links inside spoilers

A card would give away a spoiler whose text holds its link, so a preview of a link inside a spoiler
is drawn covered (`SpoilerCard`), never as one of the message's pictures:

- Which links a spoiler holds is read by `spoileredLinks` (`spoileredLinks.ts`), which parses the
  body with the plugins `Markdown` renders it with (`MESSAGE_REMARK_PLUGINS`, `remarkPipeline.ts`),
  and only when the body has a spoiler marker. A preview matches by its `url`, which the server
  writes as a WHATWG URL serializes (`normalizedUrl`).
- A preview that is a picture alone becomes a card too, with its address for a title.
- The card is blurred past reading under a Reveal spoiler button, and `inert` until the reader
  presses it: its links and play control take no pointer or focus, and assistive technology hears
  only the button. Revealing the card does not reveal the text's spoiler, nor the other way round.

## Plugins

- What the deployment's plugins say about a message shows beneath it as chips
  (`MessageAnnotations`).
- A message a plugin changed is marked "(changed by …)" beside the edited mark (`AlteredBy`).

See [plugins](../plugins/index.md).
