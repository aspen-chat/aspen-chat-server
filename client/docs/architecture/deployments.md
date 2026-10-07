# Deployments and federation

- The user's home is the server they chose and signed in to; the other deployments they use
  are signed in to from there (`Deployments`, `packages/protocol/src/deployments.ts`). The home
  lists them (`GET /users/@me/foreign-deployments`), so every device signs in to the same ones,
  and a session abroad comes from an assertion the home signs (`AspenClient.issueAssertion`)
  handed to the other deployment (`signInWithAssertion`); one that ends is replaced the same
  way, at most once per `REACQUIRE_INTERVAL_MS`. Each has its own `AspenClient`, at
  `https://{domain}`, whose session is stored per home account and domain, and its own
  `AspenSync`, which shares the home's `PreferenceStore`, since preferences are the user's and
  kept at home. `DeploymentsProvider` (`src/api/deployments.tsx`) owns them under the home's
  `SyncProvider`. `AspenClientContext` and `AspenSyncContext` are the deployment being shown:
  the home's, or, under `/at/{domain}`, that deployment's through `ForeignScope`, so every
  component that calls `useSync()` acts where it is shown; `HomeScope` puts the home back for
  what is the account's (the user footer, settings, security, bots, sign-out), and
  `SourceScope` puts one deployment in place for each entry of a list that mixes them.
  `useSources` and `useEverywhere` (`src/api/everywhere.ts`) read across every deployment: the
  rail, ordered by the account preference `RAIL_ORDER` with a globe on another deployment's
  communities, and the one DM list, ordered by `RecordStore.dmActivity`. Links carry the
  deployment (`ChannelHome.domain`, `src/features/messages/links.ts`; `useDomain`). An invite's
  shared link names its deployment, `/invite/{code}?at={domain}` (`invitePath`,
  `src/features/invites/inviteCode.ts`, made a link by `shareUrl`), so whoever opens it is taken there whatever their home:
  the invite screen goes on to `/at/{domain}/invite/{code}` when the domain is not the home's,
  and one pasted into the join form (`parseInvite`, which also reads `/at/{domain}/invite/…`)
  or posted in a message (`MessageLink`) opens the same way; a signed-out visitor is told their
  account may be on any deployment. Adding a
  server is the add dialog's last option (`OtherServerForm`), leaving one is in Settings
  (`OtherServersSection`), and signing out at home signs out everywhere (`useSignOut`). A user
  is in one call at a time across deployments (`useOneCallAtATime`, `src/api/calls.ts`), and
  the call bar shows whichever deployment's it is. Another deployment's users are shown by
  their handle `@name@domain` (`handleOf`). Before signing in at another deployment the hub
  reads its `GET /auth/methods` and signs in only where it shares a protocol version with this
  client (`CLIENT_PROTOCOL`, `commonVersion`, `packages/protocol/src/protocol.ts`); a deployment
  that shares none is `incompatible`, and `ForeignScope` says so. Code that reads what a server
  sends ignores what it does not know, as `spec/federation.md` requires: an unknown event type
  or field changes nothing in `RecordStore`, and a feature that is a capability is used only
  where `supports` says the deployment has it. A new message starts on the deployment the user
  picks (`StartOn` in the DM list's picker, the one shown by default), which hosts it. When the
  home says the user is in a DM elsewhere (`foreignDmJoined`, `AspenSync.onForeignDm`), the hub
  signs in there if needed and reads it. A block made on any deployment hides the person on
  every one (`useBlocked` with `useBlockedAnywhere`, `src/api/identity.ts`): `identityOf`
  (`packages/protocol/src/identity.ts`) names a person by their home's domain and their id
  there, from `homeDomain` and `homeId`, and `ScopeDomainContext` says which deployment a record
  in scope is from. Only the viewer's home is believed about where a user from elsewhere is
  from, since it checked the assertion the user signed in with; any other deployment's users
  from elsewhere are named by that deployment and their id there, so a deployment that claims
  its user is someone else cannot have a block of that user hide the person it named. A person
  from a third deployment blocked on a foreign one is therefore hidden on that one alone. In calls the same holds below the UI: `ShareBlocksAcrossDeployments` hands
  every deployment's sync the blocked identities (`AspenSync.setBlockedIdentities`), and its
  store's `silenced` (topic `silenced`, `useSilenced`) decides each voice's gain
  (`VoiceCall.refreshVolumes` whenever it changes) and which shared screens are hidden, even
  for someone whose record only arrives with the call.
  File offers from silenced people are hidden the same way (`FilesPanel`). File transfers in
  calls are `FileTransfers` (`packages/protocol/src/transfers.ts`), which `VoiceCall` feeds its
  signalling frames and whose state it carries as `files`; its peer connections are injected
  (`createPeerConnection`), so `transfers.test.ts` runs whole transfers between two fakes. An
  offered file's name is shown and saved without format and control characters
  (`plainFileName`, `packages/protocol/src/fileNames.ts`), so a bidirectional override cannot
  make `gpj.exe` read as `exe.jpg`; attachments' names are shown the same way. The
  repository's `docs/architecture/file-transfers.md` describes the flow. In the mobile apps a receiver chooses
  where a file goes through `AspenFilesPlugin` (Android's `android/.../files/`, iOS's
  `ios/App/App/AspenFilesPlugin.swift`, where the user picks a folder and the file is made in
  it under the name it was sent with), which `filesBridge.ts` wraps as a `FileSink`, writing
  through the bridge in base64 pieces of about a megabyte; `DocumentSinkTest` checks Android's
  native side on a device. The iOS app registers its own plugins in `MainViewController`, which `SceneDelegate` makes the window's root: the scene builds the window itself rather than from the storyboard, so a bare `CAPBridgeViewController` there would leave every plugin of the app's unimplemented.
