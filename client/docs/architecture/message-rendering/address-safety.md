# Address safety

Every address a deployment sends, or a message holds, is checked before it becomes a link, a
picture, or a window (`src/features/layout/safeUrl.ts`). **Any deployment the user holds a session
on writes some of them.**

## In the app

| Check        | Allows                                                                                                       | Used for                                                                        |
| ------------ | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------- |
| `webPageUrl` | `http:`, `https:`                                                                                            | Pages to open: a link preview's, a plugin card's or annotation's.               |
| `mediaUrl`   | `https:`; `http:` at a loopback host in a development build (`securelyReached`, see [Transport](#transport)) | Files a deployment serves: attachments, previews, icons.                        |
| `mailtoUrl`  | A `mailto:` link of the address alone, everything but its `@` encoded                                        | A person's shown email address. It cannot add recipients, a subject, or a body. |

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

| Permission                                                                    | Granted to                                         |
| ----------------------------------------------------------------------------- | -------------------------------------------------- |
| Calls, notifications, the clipboard, choosing where sound plays, saving files | The app's own page alone.                          |
| Fullscreen                                                                    | Any frame the app let ask for it (a video player). |
| Handing a link to the system (`openExternal`)                                 | Only what `externalUrl` lets out.                  |

## In the phone apps

The phone apps do the same with a Capacitor plugin that Capacitor asks about each navigation
(`AspenNavigationPlugin`, in `android/.../navigation/` and in iOS's `MainViewController.swift`):

- An address outside the app leaves it only when it is `http:`, `https:`, or `mailto:`.
- Any other is dropped.

## Transport

**The client connects to nothing, and loads nothing, over plain HTTP.** A web page it hands the
system to open may be `http:`, but what the app reaches itself is `https:` or `wss:` alone
(`securelyReached`, in `@aspen/protocol`'s `urls.ts`):

| What                                      | Where it is checked                                                                                                       |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| The deployment, entered or remembered     | `normalizeServerUrl` (`ServerForm` says why it refuses one); `defaultServerUrl` treats a remembered plain address as none |
| Every request to it, and its event stream | `AspenClient`'s constructor; `eventStreamUrl`                                                                             |
| A voice server                            | Its candidates over plain HTTP are left out of a join, untried and unblamed; with none left, the join fails               |
| Uploads to storage                        | `uploadAttachment`, `uploadIcon`                                                                                          |
| Pictures, videos, icons                   | `mediaUrl`                                                                                                                |
| The push relay                            | `RelayClient`'s constructor                                                                                               |
| The desktop app's passkey page            | `securePage` in `passkeyHandoff.ts`                                                                                       |

The shells' Content Security Policy (`shellPolicy` in `vite.config.ts`) allows the same. iOS has no
transport security exception, and Android's release build allows no cleartext.

A development build may reach this machine's own addresses (`localhost`, a name under it, a
loopback address) over plain HTTP, since its dev server and tests run there: the dev server's
build, and a shell built with `VITE_ALLOW_LOOPBACK_HTTP=1`, which `main.tsx` passes to
`allowLoopbackPlainText`. Android's debug build allows cleartext to `localhost` alone. A release
build sets neither. The desktop app's passkey hand-off returns to a listener on
`http://127.0.0.1`, which RFC 8252 prescribes for a native app's loopback redirect; nothing is
fetched from it.
