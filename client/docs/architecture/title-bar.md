# The page's title

Every shell titles the page after where the reader is.

## The page's title

`document.title` is set by `FollowDocumentTitle` (`src/features/layout/documentTitle.ts`),
mounted once at the root. Browsers show it on the tab, and the system on the window and its
taskbar button.

- Each screen names itself with `useDocumentTitle(page)`. The screen mounted last is shown;
  `null` stands for a screen still loading.
- The title is `{page} · Aspen` (`documentTitle` in `src/i18n/messages.ts`), or `Aspen` alone
  while no screen names itself (signed out, the home redirect, an invite).
- It names where the reader is no more closely than a community: never a channel, a DM's people,
  or a thread.

| Screen | Page |
| --- | --- |
| A community (`ChannelSidebarLayout`), at home or on another deployment | The community's name |
| Direct messages (`DmLayout`) | Direct messages |
| Activity, Saved messages | Their headings |
| The Administration Dashboard | Administration |
| Open source attributions | Its heading |
