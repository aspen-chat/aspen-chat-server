# Several accounts

An install may stay signed in to several accounts at once, each with its own home deployment and the
other deployments it signs in to from there. Settings' Accounts section turns it on. One account is
shown in each window, and the rest stay connected behind it.

## Where it lives

| Part                         | Where                                                                     |
| ---------------------------- | ------------------------------------------------------------------------- |
| Which accounts are held      | `AccountRegistry` (`packages/protocol/src/accounts.ts`), `aspen.accounts` |
| Their clients, the front one | `AccountsHub`, `useAccounts`, `useAllSources` (`src/api/accounts.ts`)     |
| One scope per account        | `RootLayout`, `AccountScope`, `AccountView` (`src/features/layout`)       |
| The setting and list         | `AccountsSection` (`src/features/accounts`)                               |
| Switching                    | `AccountSwitcher`, at the foot of the community rail                      |
| Adding                       | `AddAccountDialog`, with the sign-in screen's own forms                   |
| Who the message box posts as | `ActingAs`                                                                |

## What an account is

- `Account` is an id of this install's own and a server. The list holds no token; sessions are in
  the [saved sign-ins](sign-in.md#saved-sign-ins).
- The first account keeps the keys an install has always used (`aspen.session`,
  `aspen.device.<host>`). So turning the setting on moves nothing.
- Added accounts' sessions and device credentials are keyed by their local id. A home chooses the
  user ids it reports, so one must not be able to name another account's sessions. A device
  credential of its own keeps signing in from telling a deployment that two accounts share a device.
- An install holds at most `MAX_ACCOUNTS` (5).
- The web client adds accounts of its own deployment only. Its page is that deployment's code, which
  must not be given another deployment's password or sessions. Adding through an identity provider
  is not offered there, since the tab would leave.

## The account in front

`RootLayout` mounts every account under its own `AccountScope`. Each signed-in account has its
`SyncProvider` and `DeploymentsProvider`; only the front one draws the app (`ShownAccount`).

- Routes are not per account: `/communities/…` and `/at/{domain}/…` mean the account in front's, so
  shared links stay as they are.
- Each window remembers its front account in `sessionStorage`; a window with none opens on the one
  last in front. The hub remembers each account's last route for this page's life.
- Account preferences (language, motion, text size) are the front account's.
- An account behind reports no activity (`attended`), so it shows as away and its phones may be
  woken.
- An account behind that is signed out, or owes a second factor or a verified address, shows nothing
  until switched to. Then its screen shows with the switcher over it.

## Once per device, or per account

| Per account                                         | Once, over `useAllSources`                             |
| --------------------------------------------------- | ------------------------------------------------------ |
| Sync, deployments, blocks across them               | One call at a time, and the call bar                   |
| Notifications and their sounds, by its own settings | Ringing, each ring naming its account's scope          |
| Do not disturb                                      | The phone's push registration and tapped notifications |

With several held, a notification names its account, and opening one, answering a ring, or following
the call bar brings that account to the front.

## Signing out

`useSignOut` ends one account's sign-ins, at home and everywhere it signed in from there, and stops
waking the phone for those alone. Others stay signed in, and the window moves to the next. Turning
the setting off asks first, then signs out every account but the one in front.

## When access changes

Each account's event streams are its own, so what it loses reaches it as it would alone. An account
signed out by its server stays listed as signed out, and takes nothing else with it. No state is
shared between accounts but the install's device preferences.
