# Deployments and federation

An account's home is the server it signed in to. The other deployments it uses are signed in to from
there. The app shows them all together: one rail, one DM list, one call at a time. An install may
hold several accounts ([accounts.md](accounts.md)), each with its own home and deployments.

Design notes: [deployments-design-notes.md](deployments-design-notes.md).

## Where it lives

| Part                        | Code                                                                                             |
| --------------------------- | ------------------------------------------------------------------------------------------------ |
| The deployments a user uses | `Deployments`, `packages/protocol/src/deployments.ts`                                            |
| Their clients and syncs     | `DeploymentsProvider`, `src/api/deployments.tsx`                                                 |
| Reading across deployments  | `useSources`, `useEverywhere` (`src/api/everywhere.ts`)                                          |
| Protocol versions           | `CLIENT_PROTOCOL`, `commonVersion` (`packages/protocol/src/protocol.ts`)                         |
| Identities                  | `identityOf` (`packages/protocol/src/identity.ts`), `useBlockedAnywhere` (`src/api/identity.ts`) |
| One call at a time          | `useOneCallAtATime`, `src/api/calls.ts`                                                          |
| File transfers in calls     | `FileTransfers`, `packages/protocol/src/transfers.ts`                                            |
| File names                  | `plainFileName`, `packages/protocol/src/fileNames.ts`                                            |

## Signing in abroad

- The home lists the user's other deployments (`GET /users/@me/foreign-deployments`), so every
  device signs in to the same ones.
- The home tells the user's own devices when that list changes (`foreignDeploymentsChanged`, with
  the domain and whether it is now used; `AspenSync.onForeignDeployments`): a device's first sign-in
  somewhere, or its leaving. `DeploymentsProvider` hands each to `Deployments.follow`, which signs
  in there as at start, or signs out there and drops that deployment's session and store, as `leave`
  does but without asking the home, whose list already changed. A deployment this device is signing
  in to, or already uses, is left as it is; one it never used is nothing to leave.
- A session abroad comes from an assertion the home signs (`AspenClient.issueAssertion`), handed to
  the other deployment (`signInWithAssertion`).
- A session that ends is replaced the same way, at most once per `REACQUIRE_INTERVAL_MS`.

### Protocol versions

Before signing in at another deployment, the hub reads its `GET /auth/methods`. It signs in only
where the deployment shares a protocol version with this client (`CLIENT_PROTOCOL`,
`commonVersion`).

A deployment that shares none is `incompatible`, and `ForeignScope` says so.

### Reading what a server sends

Code that reads what a server sends ignores what it does not know, as `spec/federation.md` requires:

- An unknown event type or field changes nothing in `RecordStore`.
- A feature that is a capability is used only where `supports` says the deployment has it.

## One client and sync per deployment

Each deployment has:

- its own `AspenClient`, at `https://{domain}`, whose session is stored per home account and domain
- its own `AspenSync`, which shares the home's `PreferenceStore`, since preferences are the user's
  and kept at home

`DeploymentsProvider` owns them, under the home's `SyncProvider`.

### Each deployment's clock

A time a deployment stamped is judged against that deployment's clock, never the device's own, which
may be minutes or hours wrong. `AspenClient.clock` (`ServerClock`, also `AspenSync.clock`) is that
clock as this device reckons it:

- Every `ready` of the deployment's event stream carries `serverTime`. `AspenSync` notes how far it
  is from the device's clock at receipt (`ServerClock.note`), one offset per deployment.
- `clock.at(deviceTime)` and `clock.now()` add the offset to a reading of the device's clock.
- Each `ready` also saves the offset on the device (`ClockOffsetStore`, which the app keeps in
  `localStorage` under `aspen.clock.<host>`, one key per deployment, as it keeps the device
  credential; `clockStore` in `src/api/devices.ts`). It outlives signing out.
- A run starts from the saved offset (`ServerClock.restored`), so what is judged before its first
  `ready` (the sign-in screen's ceremonies and codes, a restored session's token) is corrected on a
  device whose clock is wrong. The home's client, signed in or not, and each other deployment's
  client are given their own. The first `ready` replaces it.
- A saved value that is not a finite number, or is further than `CLOCK_OFFSET_MAX_MS` (30 days) from
  zero, is ignored.
- The offset is zero where none is saved, and for a deployment whose `ready` gives no time: the two
  clocks are then taken to agree.
- It is late by the time `ready` spent on its way, and no more.

What is judged by it:

| Judgement                                                                      | Where                                                 |
| ------------------------------------------------------------------------------ | ----------------------------------------------------- |
| Whether the session token is about to expire                                   | `AspenClient` (`sessionTokenExpiresSoon`)             |
| How long a conditional passkey sign-in's ceremony has left                     | `runConditionalPasskeySignIn`                         |
| Whether a connection's replay covers the bootstrap (`replayedSince`)           | `AspenSync`'s `#replayMayLack`                        |
| Whether a message was posted since the sync began, and so is news              | `AspenSync`'s `#onEvent`                              |
| Whether an invite has expired, and the expiry a new or changed invite is given | `lookupInvite`, `InviteDialog`, `RegistrationInvites` |
| When a timed mute or presence override ends                                    | `#scheduleMuteEnd`, `#scheduleChosenPresenceEnd`      |
| Whether a ring is still in force                                               | `ringing` (`features/voice/ringing.ts`)               |
| Whether a poll still takes votes, and how long it has left                     | `usePollNow`                                          |
| Whether a ban has ended                                                        | `BannedList`                                          |
| How long a sign-in code has left                                               | `DeviceLinkCode`                                      |

A timed mute or override is not ended before this run's first `ready` has said what the clock is
(`ServerClock.heard`, which an offset saved by an earlier run does not set), since one dropped by a
wrong clock stays dropped until the next read. New code that compares a time the server stamped with
now reads `clock.at` or `clock.now`, not `Date.now()`. Times this device stamped itself (a file
offer's expiry, a mana level's receipt, who is typing) stay on its own clock.

## Scopes

`AspenClientContext` and `AspenSyncContext` are the deployment being shown. Every component that
calls `useSync()` acts where it is shown.

| Scope          | Puts in place                                                                                  |
| -------------- | ---------------------------------------------------------------------------------------------- |
| (default)      | The home                                                                                       |
| `ForeignScope` | Under `/at/{domain}`, that deployment                                                          |
| `HomeScope`    | The home again, for what is the account's: the user footer, settings, security, bots, sign-out |
| `SourceScope`  | One deployment for each entry of a list that mixes them, under that source's account           |

## Reading across deployments

`useSources` and `useEverywhere` read across every deployment. They feed:

- the rail, ordered by the account preference `RAIL_ORDER`, with a globe on another deployment's
  communities
- the one DM list, ordered by `RecordStore.dmActivity`

Another deployment's users are shown by their handle, `@name@domain` (`handleOf`).

## Links and invites

Links carry the deployment (`ChannelHome.domain`, `src/features/messages/links.ts`; `useDomain`).

An invite's shared link names its deployment: `/invite/{code}?at={domain}`. It is made by
`invitePath` (`src/features/invites/inviteCode.ts`) and made a link by `shareUrl`. Whoever opens it
is taken there, whatever their home:

- The invite screen goes on to `/at/{domain}/invite/{code}` when the domain is not the home's.
- One pasted into the join form (`parseInvite`, which also reads `/at/{domain}/invite/…`) opens the
  same way.
- One posted in a message (`MessageLink`, `namedInvite`) opens the same way.

`parseInvite` reads a pasted link whole (`routeAddress`; see
[Reading a link](qr-codes.md#reading-a-link)), its invite route being the link's whole path:

| Pasted                                                | Read as                             |
| ----------------------------------------------------- | ----------------------------------- |
| A bare code                                           | The deployment shown's              |
| A link with `?at=`, or `/at/{domain}/invite/{code}`   | The named deployment's, on any host |
| `/invite/{code}` on one of the home's own addresses   | The deployment shown's              |
| `/invite/{code}` on any other host                    | The deployment's at that host       |
| `/invite/{code}` as an `aspen:` link or an app's page | The deployment shown's              |

In a message, only a link that names its deployment is an invite (`namedInvite`). **Why:** a link
that merely has an invite's path on some other host is usually another site's own, and opens as any
link does.

- A signed-out visitor is told their account may be on any deployment.

## Adding, leaving, and signing out

| Action       | Where                                                                 |
| ------------ | --------------------------------------------------------------------- |
| Add a server | The add dialog's last option (`OtherServerForm`)                      |
| Leave one    | Settings (`OtherServersSection`)                                      |
| Sign out     | Signing an account out at home signs it out everywhere (`useSignOut`) |

The join form (`JoinForm`: the add dialog's join step, and the empty home screen's) hands a
registration invite of another deployment, scanned or pasted, to the add-a-server form
(`routeJoinLink`):

- The form opens with the deployment's domain and the invite filled in, and the registration invite
  field showing, which otherwise appears only once the deployment answers that it requires one.
- Nothing is sent until the reader presses Sign in.
- In the add dialog the join step gives way to the server step, whose back arrow leads to the choice
  and forgets what was filled in. On the empty home screen the form opens in a dialog of its own
  (`OtherServerDialog`).
- Where the home takes no part in federation there is no such form, and the invite is refused,
  naming its deployment.

## DMs

- A new message starts on the deployment the user picks (`StartOn` in the DM list's picker; the one
  shown by default). That deployment hosts it.
- When the home says the user is in a DM elsewhere (`foreignDmJoined`, `AspenSync.onForeignDm`), the
  hub signs in there if needed and reads it.

## Calls

- A user is in one call at a time across deployments (`useOneCallAtATime`).
- The call bar shows whichever deployment's call it is.

## Blocks across deployments

A block made on any deployment hides the person on every one (`useBlocked` with
`useBlockedAnywhere`). See also [Blocking](blocking.md).

### Identities

- `identityOf` names a person by their home's domain and their id there, from `homeDomain` and
  `homeId`.
- `ScopeDomainContext` says which deployment a record in scope is from.

### Whose word is believed

- Only the viewer's home is believed about where a user from elsewhere is from, since it checked the
  assertion the user signed in with.
- Any other deployment's users from elsewhere are named by that deployment and their id there.
- So a person from a third deployment, blocked on a foreign one, is hidden on that one alone.

**Why:** a deployment that claims its user is someone else must not be able to have a block of that
user hide the person it named. See
[the design notes](deployments-design-notes.md#whose-word-is-believed-about-identity).

### In calls

The same holds below the UI:

1. `ShareBlocksAcrossDeployments` hands every deployment's sync the blocked identities
   (`AspenSync.setBlockedIdentities`).
2. That store's `silenced` (topic `silenced`, `useSilenced`) decides:
   - each voice's gain (`VoiceCall.refreshVolumes` whenever it changes)
   - which shared screens are hidden, even for someone whose record only arrives with the call
   - which file offers are hidden (`FilesPanel`)

## File transfers in calls

`FileTransfers` handles file transfers in calls:

- `VoiceCall` feeds it its signalling frames and carries its state as `files`.
- Its peer connections are injected (`createPeerConnection`), so `transfers.test.ts` runs whole
  transfers between two fakes.

The repository's `docs/architecture/file-transfers.md` describes the flow.

### File names

An offered file's name is shown and saved without format and control characters (`plainFileName`).
Attachments' names are shown the same way. **Why:** a bidirectional override must not make `gpj.exe`
read as `exe.jpg`.

### Saving on mobile

In the mobile apps, a receiver chooses where a file goes through `AspenFilesPlugin`:

| Platform | Code                                 | Behaviour                                                                             |
| -------- | ------------------------------------ | ------------------------------------------------------------------------------------- |
| Android  | `android/.../files/`                 | `DocumentSinkTest` checks the native side on a device                                 |
| iOS      | `ios/App/App/AspenFilesPlugin.swift` | The system's save picker moves an empty file where the user says; it is written there |

`filesBridge.ts` wraps the plugin as a `FileSink`, writing through the bridge in base64 pieces of
about a megabyte.

### iOS plugin registration

The iOS app registers its own plugins in `MainViewController`, which `SceneDelegate` makes the
window's root. The scene builds the window itself rather than from the storyboard.

**Warning:** a bare `CAPBridgeViewController` as the root would leave every one of the app's plugins
unimplemented.
