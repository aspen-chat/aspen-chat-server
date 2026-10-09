# Message bodies, attachments, and video links

How a message is drawn: its Markdown body, links, attachments and their previews, link and video
cards, and how runs of one author's messages are grouped.

## Pages

- [Markdown](markdown.md): rendering, nesting and table limits, error fallbacks, which links become
  links, code highlighting, spoilers, and linkifying bare addresses.
- [Links to a deployment](self-links.md): chips that name what a link to a deployment the user uses
  leads to.
- [Attachments](attachments.md): uploading, inline pictures and the gallery, previews and posters,
  and descriptions.
- [Link and video cards](link-cards.md): video players, link preview cards, and plugins' chips.
- [Address safety](address-safety.md): which addresses become links, pictures, or windows, in the app
  and in the desktop and phone shells.
- [Message groups](message-groups.md): drawing a run of one author's messages as a group.
- [Design notes](design-notes.md): why rendering works this way.

## Key files

Paths are under `src/features/` unless they name a package.

| Part | Where |
| --- | --- |
| Markdown | `messages/Markdown.tsx`, `messages/markdownLimits.ts`, `messages/remarkLiteralLinks.ts`, `messages/remarkSpoilers.ts`, `messages/Spoiler.tsx` |
| Code highlighting | `messages/highlighter.ts`, `messages/CodeBlock.tsx` |
| Linkifying | `messages/linkify.ts` |
| Self links | `messages/selfLinks.ts`, `messages/SelfLink.tsx` |
| Attachments | `messages/Attachments.tsx`, `messages/ImageGallery.tsx`; uploading in `packages/protocol/src/upload.ts` |
| Cards | `messages/VideoCard.tsx`, `messages/video.ts`, `LinkPreviewCard` in `messages/MessageBody.tsx` |
| Error fallback | `layout/ErrorBoundary.tsx` |
| Address checks | `layout/safeUrl.ts`; desktop `packages/desktop/src/main/navigation.ts` |
| Grouping | `messages/messageGroups.ts`, `useGroupContinuations` in `MessageList` |
| Styles | `message-body` rules and `code-*` tokens in `styles.css` |
