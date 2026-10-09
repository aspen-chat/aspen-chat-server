# Address safety

Every address a deployment sends, or a message holds, is checked before it becomes a link, a
picture, or a window (`src/features/layout/safeUrl.ts`). **Any deployment the user holds a session
on writes some of them.**

## In the app

| Check | Allows | Used for |
| --- | --- | --- |
| `webPageUrl` | `http:`, `https:` | Pages to open: a link preview's, a plugin card's or annotation's. |
| `mediaUrl` | `https:`; `http:` at a loopback host, or while the page is itself served over plain HTTP (as a development deployment is) | Files a deployment serves: attachments, previews, icons. |
| `mailtoUrl` | A `mailto:` link of the address alone, everything but its `@` encoded | A person's shown email address. It cannot add recipients, a subject, or a body. |

- An attachment at any other address is unavailable.
- A preview's picture at any other address is left out.
- In message text, only absolute `http:`, `https:`, and `mailto:` addresses become links (see
  [Markdown](markdown.md#which-links-become-links)).

## In the desktop shell

The desktop shell checks again (`packages/desktop/src/main/navigation.ts`):

- The window navigates only to the app's own `index.html` (or the dev server).
- It hands the system only `http:`, `https:`, and `mailto:` links.

Its session grants permissions (`permitted` there) only as the app needs them, and nothing else to
anyone:

| Permission | Granted to |
| --- | --- |
| Calls, notifications, the clipboard, choosing where sound plays, saving files | The app's own page alone. |
| Fullscreen | Any frame the app let ask for it (a video player). |
| Handing a link to the system (`openExternal`) | Only what `externalUrl` lets out. |

## In the phone apps

The phone apps do the same with a Capacitor plugin that Capacitor asks about each navigation
(`AspenNavigationPlugin`, in `android/.../navigation/` and in iOS's `MainViewController.swift`):

- An address outside the app leaves it only when it is `http:`, `https:`, or `mailto:`.
- Any other is dropped.
