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

## Plugins

- What the deployment's plugins say about a message shows beneath it as chips
  (`MessageAnnotations`).
- A message a plugin changed is marked "(changed by …)" beside the edited mark (`AlteredBy`).

See [plugins](../plugins/index.md).
