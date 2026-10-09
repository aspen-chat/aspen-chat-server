# Links to a deployment

A link to a deployment the user uses shows as a chip naming what it leads to, rather than its
address.

| Part | Where |
| --- | --- |
| Parsing | `parseSelfLink` in `src/features/messages/selfLinks.ts` |
| The chip | `SelfLink.tsx` |
| Opening it | `selfLinkRoute` |

## Which links

A deployment the user uses is:

- the home, at the address the app reaches it at or the page's own;
- every deployment signed in to from there.

Every route of the app is read:

- a deployment's front page, communities, channels, messages, threads, DMs, and invites, on its own
  address or the home's under `/at/{domain}` (an invite's `?at=` naming its deployment);
- the home's own pages: the dashboard and its tabs, adding a bot, registration invites, sign-in
  codes, attributions.

The deployment's API, files, and any other path stay ordinary links.

## What the chip shows

The chip shows the name behind each of the link's ids, outside in, between Phosphor carets, such as
`Family › general › Message from Bob`. It is led by the deployment's domain when that is not the one
the message is from.

Names come from the store of the deployment the link is to (`SourceScope`). Wherever a name is
unknown or not the reader's to see, the chip uses a word for the kind of thing (`unnamed`).

| Thing | Named from |
| --- | --- |
| Community, channel | The store. |
| DM | Its people, read on demand with `AspenSync.ensureChannel`, with a skeleton meanwhile. |
| Message | Its author, as the read of the linking message sideloaded it (`include=linked`). |
| Thread | The word "Thread", since a thread's starter message names it. |
| Invite | Its code. Reading what an invite opens counts against the limit on guessing invites. |

## Using the chip

- Pressing the chip opens the route here through the router (`selfLinkRoute`), so the page is not
  loaded again.
- Its tooltip is its address.
- Right-clicking it, or holding a finger on it, opens a menu that copies the address or opens it as
  an ordinary link would. The hold has its own timer, which keeps the press from the message's
  actions.
