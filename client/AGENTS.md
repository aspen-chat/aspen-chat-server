# Aspen client — agent instructions

This directory is a pnpm workspace containing the cross-platform Aspen client. The root
`AGENTS.md` applies here in full (comments describe the current code, seek clarification, do not
commit to `main`). The rules below are specific to the client.

## Layout and dependency direction

```
packages/protocol   generated types + HTTP/session client + event stream   (no UI, no React)
packages/app        React + React Aria UI                                  (depends on protocol)
packages/desktop    Electron main/preload                                  (loads app's build; imports nothing from app)
packages/mobile     Capacitor config                                       (wraps app's build)
```

`protocol` must stay free of React and of any platform API so it can be unit-tested in Node and
reused by any shell. `app` must stay free of Node and Electron APIs: it is a web page, and in
Electron it runs sandboxed with `contextIsolation`. Anything the page needs from the host goes
through the preload bridge (`window.aspenDesktop`), whose shape is declared in
`packages/app/src/vite-env.d.ts`.

## Icons

Every icon the apps ship is drawn by `scripts/export_icons.py` from one definition of the canopy
mark, in two levels of detail: full (bark marks, fine gaps between the leaves) from 64 pixels up,
and small (no bark marks, a darker and thicker trunk, wider gaps) below, since fine detail turns
to mud at favicon sizes. The gaps are cut out rather than painted, so the mark sits on any
background. Wherever an icon may be transparent (favicons, the desktop icons, Android's legacy
launcher icons) it is the bare mark, its trunk a shade darker so it shows on a light panel too;
where the platform needs a solid one (the iPhone's home screen, Android's adaptive icon and
splash, macOS's Dock) the mark sits on charcoal, since a pale tile swallows the pale trunk. It
writes the brand art (`brand/`: the mark, the icon tile, and the wordmark side by
side and stacked, each for light and dark grounds), the web client's `favicon.svg`,
`favicon.ico`, and `apple-touch-icon.png` (`packages/app/public/`), the desktop app's icons
(`packages/desktop/build/`: sized PNGs for Linux, which also give the window its icon there, an
`.ico` for Windows, and a 1024 PNG on macOS's icon grid, from which electron-builder makes the
`.icns`), the Android launcher icons (legacy, round, and adaptive, with a monochrome layer for
themed icons), the status bar icon `ic_stat_aspen` that push notifications use, the splash
images, and the favicon inlined in the server's passkey page. Change the mark there and run it
again; never edit its outputs by hand. It needs `rsvg-convert` and ImageMagick's `magick`.

## The server contract is generated, never hand-written

`packages/protocol/src/generated/` is produced by `pnpm codegen` from `../openapi.yaml` and
`../event_schema.json`, which the server produces with `cargo run -- --gen-openapi-schema`. Do not
edit generated files, do not declare server types by hand, and do not build a request URL as a
string. Use the typed openapi-fetch client (`client.api.GET("/api/v1/channels/{channel}", …)`)
so a server-side change surfaces as a compile error here.

When the server API changes, run `pnpm codegen:regen` and fix whatever stops compiling.

## Talking to the server

- Every request goes through `AspenClient` (`packages/protocol/src/http.ts`). It attaches the
  bearer token, refreshes the session token before it expires and on a `401`, and replays the
  request once. Do not call `fetch` directly for API traffic.
- Signing in: `AspenClient.login` answers `signedIn` or `secondFactorRequired` with a ticket,
  finished by `completeSecondFactor` or a passkey; `runPasskeyCeremony` runs any passkey
  ceremony (`signIn`, `register`, `reauthenticate`) end to end and stores the session a
  `signIn` produces. A ceremony reaches an authenticator by a `PasskeyTransport`
  (`packages/protocol/src/passkeys.ts`), chosen in `src/features/auth/passkeyTransport.ts`: the
  web client runs it in its own page when served under the server's `rpId` and offers no
  passkeys otherwise; the desktop and mobile shells hand it to the server's `/auth/passkey` page
  in the system browser, never running one in their own page. On the desktop the main process
  opens that page and listens on a one-shot loopback port for the browser's return
  (`packages/desktop/src/main/passkeyHandoff.ts`), then sends the tab back to the page to say
  it is done. On mobile the return is the `aspen://auth/passkey` URL, caught with
  `@capacitor/app` while `@capacitor/browser` shows the page, so the native projects register
  the `aspen` URL scheme (an intent filter on Android, `CFBundleURLTypes` on iOS once its
  project is made); this path has not yet run on a device.
- A server that requires two-factor sign-in answers every request of an account without a
  second factor with `twoFactorEnrollmentRequired`. `AspenClient` notices it on any response
  and flags the session (`twoFactorEnrollmentRequired`), and the root layout then shows the
  enrollment screen instead of the app until a factor is added and its recovery codes have
  been dismissed (`markEnrolled`). Security changes the server says need a fresh verification
  go through `useReauth()` (`src/features/security/reauthContext.ts`), which asks the user to
  confirm it's them and tries once more, and resolves to `undefined` when they decline, so an
  action that has no result of its own returns one to tell success apart. Sign-in and security
  (`SecurityPanel`) changes the password too (`PUT /users/@me/password`, which signs out every
  other session), for users of this deployment only. A form that marks a field invalid clears
  that error as soon as the field is edited, since an invalid React Aria field blocks the
  form's next submission.
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
  shared link names its deployment, `/invite/{code}?at={domain}` (`shareableInviteLink`,
  `src/features/invites/inviteCode.ts`), so whoever opens it is taken there whatever their home:
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
  in scope is from. In calls the same holds below the UI: `ShareBlocksAcrossDeployments` hands
  every deployment's sync the blocked identities (`AspenSync.setBlockedIdentities`), and its
  store's `silenced` (topic `silenced`, `useSilenced`) decides each voice's gain
  (`VoiceCall.refreshVolumes` whenever it changes) and which shared screens are hidden, even
  for someone whose record only arrives with the call.
  File offers from silenced people are hidden the same way (`FilesPanel`). File transfers in
  calls are `FileTransfers` (`packages/protocol/src/transfers.ts`), which `VoiceCall` feeds its
  signalling frames and whose state it carries as `files`; its peer connections are injected
  (`createPeerConnection`), so `transfers.test.ts` runs whole transfers between two fakes. The
  root AGENTS.md's File transfers describes the flow. In the mobile apps a receiver chooses
  where a file goes through `AspenFilesPlugin` (Android's `android/.../files/`, iOS's
  `ios/App/App/AspenFilesPlugin.swift`, where the user picks a folder and the file is made in
  it under the name it was sent with), which `filesBridge.ts` wraps as a `FileSink`, writing
  through the bridge in base64 pieces of about a megabyte; `DocumentSinkTest` checks Android's
  native side on a device. The iOS app registers its own plugins in `MainViewController`.
- The iOS project (`packages/mobile/ios`) is kept, as Android's is. Its UI tests
  (`ios/App/AppUITests`, scheme `App`) drive the app with real taps and finger drags in WebKit
  against the server the web build names, signing in as `ASPEN_TEST_USER`/`ASPEN_TEST_PASSWORD`
  (given to `xcodebuild test` as `TEST_RUNNER_`-prefixed variables; without them they skip):
  `testReadingBackMovesOnlyWithTheFinger` is Safari's own answer to `historyScroll.spec.ts`.
  The Administration Dashboard's user directory shows a user of another deployment as
  `name@domain`, and offers moderators a ban from this deployment (`BanForeignUser`,
  `AspenSync.setForeignUserBanned`).
- Every endpoint is rate limited and may answer `429` `rateLimited` with `Retry-After`.
  `AspenClient` retries a read once when the wait is at most `RATE_LIMIT_RETRY_MAX_MS`; a
  refused write, or a longer wait, reaches the caller as an `ApiProblemError` whose localized
  text says to slow down. Code that polls must stay well inside the server's built-in limits
  (`server/src/rate_limits.toml`), as presence polling does.
- Errors are RFC 9457 Problems. Branch on `problem.code`, never on `title` or `detail`, which are
  localized prose for display. `unwrap()` converts a failed openapi-fetch result into an
  `ApiProblemError` for callers that prefer exceptions.
- The event stream (`packages/protocol/src/events.ts`) authenticates with an `identify` first
  frame, tracks the sequence of the last event it delivered, and asks the server to resume from
  it on reconnect. When it reports `onResyncRequired`, the server could not replay the gap:
  throw away cached state and re-bootstrap from REST before applying further events. Supply
  `authenticate` from `AspenClient.freshSessionToken` so a rejected token is refreshed rather
  than retried.
- Reads that accept `include` (community reads, the user's community list, message reads) return
  `{ data, included }` whether or not you asked for anything, and `included` holds the same
  record types the event stream carries, keyed by type (`channels`, `users`, ...). Pass
  `query: { include: ["channels", "members"] }`; the client serializes arrays comma separated
  because that is how the OpenAPI document declares them. Prefer one read with `include` over a
  fan-out of per-community or per-message requests.
- Server state lives in `RecordStore` (`packages/protocol/src/store.ts`), a normalized cache
  keyed by record type and id, kept current by `AspenSync` (`sync.ts`): REST bootstrap first,
  then the event stream, whose replay window covers the gap. Components read the store through
  the hooks in `src/api/hooks.ts`, each subscribed to one store topic; never copy server records
  into component state or fetch them with `client.api` directly from a component. New reads and
  writes go in `AspenSync`, and new record types go in the store with their event handling.
- A write's REST response never overwrites a record the stream already holds. The server
  publishes events before it answers, so an event that arrives while the request is in flight
  is newer than the response; the response only fills in a record the stream has not delivered
  yet. Updates to existing records are left to their events entirely.
- Message bodies are GitHub-flavoured Markdown, rendered by `src/features/messages/Markdown.tsx`
  with `react-markdown` (no raw HTML, unsafe schemes dropped); element styles are the
  `message-body` rules in `styles.css`. Fenced code is highlighted by highlight.js
  (`src/features/messages/highlighter.ts`), which loads as its own chunk on the first code
  block: its "common" grammars come with that chunk and every other grammar it ships is fetched
  on first use. Token colours are the `code-*` palette tokens, mapped from `hljs-*` classes at
  the end of `styles.css`. Nothing is auto-detected; an unlabelled fence is plain. Spoilers are
  `||text||` (Discord) or `>!text!<` (Reddit), wrapped by `remarkSpoilers.ts` and rendered by
  `Spoiler.tsx` as a block the reader activates to reveal; markup inside them is kept. On top of
  GFM's own autolinks, bare domains are linked by
  the tokenizer in `src/features/messages/linkify.ts`: explicit
  `http(s)` URLs, and bare domains whose TLD is on IANA's list (the `tlds` package). A handful
  of TLDs that double as source-file extensions only link with a port, a path, or `www.`; the
  set is a constant in that file. The server's preview extractor
  (`server/src/app/link_preview.rs`, list in `tlds.txt`) applies the same rule, so what renders
  as a link is what gets a preview; change both together.
- Attachments upload in the server's two phases from `AspenSync.uploadAttachment` (reserve,
  `PUT` the bytes straight to storage with `uploadFetch`, confirm) and are named by id in
  `sendMessage`. A picture's reservation carries its size, which the composer measures first
  (`measurePicture`), and its record gives it back as `width` and `height`. A message event carries only ids, so `useAttachment` fetches records on
  demand. Images render inline (`src/features/messages/Attachments.tsx`): image attachments,
  links whose path has an image extension, and links the server found to be images, which
  arrive as previews with a picture and no text. `MessageMedia` there gathers all three into
  one strip and shows at most `INLINE_IMAGE_LIMIT` (three) inline; beyond that a `+N` tile,
  like any inline picture, opens `ImageGallery.tsx`, a modal that pages through the whole set.
- Video links get a card with a play control (`src/features/messages/VideoCard.tsx`). The
  server only sends a player for providers in its `VIDEO_PROVIDERS` table
  (`server/src/app/link_preview.rs`), and `src/features/messages/video.ts` keeps the matching
  list of player hosts the client will frame; extend both together. Twitch's player needs the
  embedding hostname as `parent`, which `playerSrc` adds. The player iframe is sandboxed and
  only created after the reader presses play.
- Polls (`src/features/messages/PollCard.tsx`, `CreatePollDialog.tsx`, `PollClosedNotice.tsx`)
  are records of their own: a message of kind `poll` names one in its `poll` field and carries
  no text, and a message of kind `pollClosed` is the announcement the server posts when the
  deadline passes. The tally lives in the poll record's `results` and is republished as an
  update event on every vote, so a vote only records the caller's own choice locally
  (`store.setMyVote`) and leaves the numbers to the stream. Message reads sideload `polls` with
  the caller's `pollVotes`, which is the only way to learn one's own vote on an anonymous poll;
  `usePoll` fetches a poll the window did not bring. Each option is `{ label, emoji? }`; the
  emoji is chosen in the dialog from the same lazily loaded picker reactions use, and the server
  validates it as a single emoji. The outcome text of the announcement is
  composed on the client from the final tally (`src/features/messages/poll.ts`), so it is
  localized like everything else. The card offers Close poll to the poll's creator and to a
  holder of Manage messages while it is open (`AspenSync.closePoll`), whose closing comes as
  the poll's update and the announcement, like a deadline's.
  A poll whose creator allowed write-ins lists its `writeIns` after its `options`, and both
  share one index space that votes use: write-in `i` is answer `options.length + i`, and a
  removed one stays as `null` so no index moves (`pollChoices`, `choiceAt`). Each write-in
  notes who wrote it in, or only that it was written in on an anonymous poll. Reads with votes
  also sideload `ownWriteIns`, the caller's own standing write-ins (`store.myWriteIns`), which
  decide whether the card offers the field for one (one each) and a remove control, offered
  to the writer and the poll's creator. Writing in an answer the poll already has votes for
  it instead (`sync.writeIn` resolves to the answer voted for). A poll's `update` event that
  nulls an answer also drops the caller's vote and write-in on it, since anyone permitted may
  remove one.
- Reordering is drag and drop from React Aria (`useDragAndDrop` on a `GridList` for the
  community rail and one per channel group, each row carrying a `Button slot="drag"` handle
  that keyboard and screen reader users drag with), with `src/features/layout/reorder.ts`
  turning a drop into the new id order. `AspenSync.reorderCommunities` and `reorderChannels`
  renumber positions, patch only the records whose index changed, and arrange the cache at
  once so the events that follow are no-ops. Community order is per user: it lives on the
  caller's membership (`UserCommunity.sortIndex`), which every community read sideloads.
  A channel dragged into another group, or onto an empty category's drop zone, moves there:
  `arrangeChannels` sets its `parentCategory` along with its position. Rows carry a private
  drag type so channel groups accept only channels and the rail accepts only communities.
  Categories keep their own order for now.
- Communities gather into folders on the rail, as apps do on an iPhone's home screen
  (`CommunityRail`, `RailFolder.tsx`). Folders are the account preference `RAIL_FOLDERS`
  (`rail.folders`: each folder's id, name, tint from `FOLDER_COLORS`, whether it is open, and
  its members' rail keys in order), and `RAIL_ORDER` names a folder where it stands as
  `folder:{id}`; the two are written together in one request (`PreferenceStore.setAccount`),
  and a client that knows nothing of folders shows their communities at the end of its rail.
  The rail stays one `GridList`, an open folder's communities following its row on its tint,
  so pointer, touch, and keyboard dragging and screen readers work the same throughout. What
  a drop means is `moveInRail` (`railOrder.ts`, whose tests list every case): a community
  dropped on another makes a folder of the two, on a folder or one of its communities joins
  it, among an open folder's communities goes in there, and after its last comes out; a folder
  moves whole and never onto anything; a folder left with one community goes. A closed folder
  shows its first four icons two by two and speaks for its communities (unread mark, tag
  count, the current community's ring); pressing it opens it in place. Its menu, from a right
  click or its options button, renames, tints, and ungroups it. Each deployment still gets its
  own share of the order, folders opened in place (`railSequence`).
- Icons (user avatars and community icons) are uploaded through `AspenSync.uploadIcon`, the
  server's two-phase flow, and then named by id on the user or community. `IconPicker.tsx` in
  `src/features/media` runs the whole thing: file chooser, the `CropDialog` where the reader
  places a circle that never leaves the picture (geometry in `crop.ts`), a square PNG crop of
  that circle at most `ICON_MAX_SIZE` wide, upload, and the id. `Avatar` takes an `iconId` and
  shows the picture once `useIcon` has the record, initials until then.
- Profiles are fields on the user record (`displayName`, `pronouns`, `bio`, `status` as
  `{ text, emoji? }`), edited by `AspenSync.updateProfile` as a merge patch built by
  `src/features/users/profile.ts`, which also decides what to call a user (`displayNameOf`).
  Show that name wherever a user is named, never `name` directly; `ProfileCard.tsx` is the card
  any user control opens, and `EditProfileDialog.tsx` the signed-in user's editor. Opened within
  a community (a `communityId` in the route) the card lists the roles the user holds there,
  read with `AspenSync.loadMember` when they are outside the member sample, and offers those
  who may change them the ones they could give and an × on each they could take away
  (`useAssignableRoles`, which the members panel's role picker shares).
- Voice lives in `packages/protocol/src/voice.ts`. `AspenSync.voice` is a `VoiceCall`: `join(channelId)`
  asks the API server for a join offer (`POST /channels/{channel}/voice/join`), opens the
  microphone (a refusal fails the join with `errorKind: "microphone"` before any server is
  tried or blamed; browsers refuse media on an insecure origin, which is anything but `https`
  or `localhost`), pings each candidate's `/health` at once, and tries them nearest first; a candidate that refuses the token
  or does not answer within `READY_TIMEOUT_MS` is reported to `POST /voice-servers/{server}/failures`
  and the next one is tried. Once a server answers `ready`, the call loads a mediasoup `Device`,
  opens a send and a receive transport, produces the microphone, and then waits for the send
  transport's ICE and DTLS to connect (`CONNECT_TIMEOUT_MS`): the server accepting the producer
  says nothing about media, and a transport that fails or times out counts as that server's
  failure, so it is reported and the next candidate tried. A transport that fails mid-call
  rejoins. Every `newConsumer` the server announces is consumed: audio is played through a hidden element, video is a `RemoteScreen` in `state.screens` for the app to render. `startScreenShare()` asks `VoiceMedia.getScreen()` (`getDisplayMedia` with audio, at the screen's own resolution up to 4K and 60 frames a second, `SCREEN_QUALITY`) and produces the picture as `screen` (one layer allowed up to 25 Mbps at 60 fps, starting at 10 Mbps rather than climbing from a few hundred kbps: `SCREEN_ENCODING`, `SCREEN_VIDEO_CODEC`; the network's bandwidth estimate, not a guess of ours, brings it down) and any sound as `screenAudio`, the sound as stereo Opus at 128 kbps without DTX (`SCREEN_AUDIO_CODEC`; mediasoup-client sends mono unless told, and a listener decodes whatever the producer declares); its `contentHint` option marks a picture that moves, such as a game, so the encoder gives up resolution rather than frames when bandwidth runs short; `stopScreenShare()` closes both, and the share also ends when the browser's own stop control ends the track. `state.sharingScreen` and `state.localScreen` (the preview track) describe the user's own share. `startCamera()` asks `VoiceMedia.getCamera()` for the camera chosen in Settings (`video.input`, a device preference, up to 1080p at 30 frames a second, `CAMERA_QUALITY`) and produces it as `camera` (up to 4 Mbps, `CAMERA_ENCODING`), shown to the user as `state.localCamera` and moved to another camera when the choice changes; `stopCamera()` ends it. Others' cameras are `state.cameras`, apart from `state.screens`, and the call screen shows each above its owner's name in place of the avatar, the tile four columns wide where they fit and the whole row where not, the user's own mirrored; a camera goes full screen by its corner button or a double click (`ScreenTile`, without the F key, which means the shared screen). `state.canCamera` says whether the join offer allowed one (Use camera). `getCamera` tries the chosen camera and then every other, so one held by another app does not leave the user without the rest; a failure is a `CameraError` whose `failure` says why (`none` connected, access `denied`, every camera `failed`, or the voice server did not take it, `unsent`), kept as `state.cameraError` until the camera turns on, `clearCameraError()`, or the call ends, and the call bar shows it under its buttons with what to do. The Android app declares `CAMERA` (and the camera as optional hardware) so its WebView may ask for one. In Electron, `getDisplayMedia` only works because the main process answers it in `setDisplayMediaRequestHandler` (`packages/desktop/src/main/index.ts`), with a picker as described under screen sharing on the desktop shell below. The signalling frames are the generated
  `src/generated/voiceSignal.ts` (from `voice_signal_schema.json`, which `pnpm codegen` builds by
  running the voice server with `--gen-signal-schema`). `VoiceCallState` is read with
  `useVoiceCall()`; it keeps `channelId` while `failed` so the call bar can show why. A call
  whose session ends with reason `serverLost` or `serverRemoved` (from the `voiceSessionEnded`
  event, which `AspenSync` hands to the call), or whose socket drops unannounced, rejoins after a
  random pause of at most `REJOIN_DELAY_MAX_MS` (a second); an `idle` ending sets `endedReason`,
  which `VoiceEndedDialog` shows until `acknowledgeEnd()`; an `empty` ending is the user's own
  leave. Browser media (`getUserMedia`, hidden `<audio>` elements per consumer) is in
  `browserMedia.ts` behind the `VoiceMedia` interface, loaded lazily so the protocol package
  stays importable in Node, and tests pass a fake. Who is in each channel's call is store state
  (`RecordStore.channelVoice`, topic `voice:<channelId>`, hook `useChannelVoice`), built from the
  `voice` sideload and the `voiceSession`, `voiceParticipant`, and `voiceSpeaking` events; each
  participant carries `speaking` and `lastSpokeAt`, and `src/features/voice/voiceList.ts` picks
  the fifteen to show under a channel (most recent speakers first once a call is larger than
  that). `VoiceParticipants` draws them with a green ring while speaking and a monitor mark while
  sharing. Clicking a person shows their `ProfilePopover` beside the row (anchored to the row,
  not the name); clicking them again or anywhere else closes it. Right-clicking a person, or
  the dots beside them, opens `ParticipantMenu`: how loud they are to this user alone (a gain
  from 0 to `MAX_USER_VOLUME`, the `userVolume(userId)` device preference), "mute for me"
  (the `userMuted(userId)` device preference, which silences them for this user while keeping
  their volume, nothing the server or they can see), and, while they share a screen, the same
  two for its sound apart from their voice (`streamVolume`, `streamMuted`, through
  `setStreamVolume` and `setStreamMuted`), and the moderation actions. Voice and stream go
  through `AspenSync.setUserVolume` and `setUserMuted` and their stream counterparts, which
  apply `effectiveUserVolume` or `effectiveStreamVolume` to that person's `microphone` or
  `screenAudio` consumers alone (`VoiceCall` asks `userVolume(user, source)`); playback runs through a Web Audio gain node because an
  element's own volume stops at 1. Chromium, and so the desktop app, feeds a remote WebRTC
  track into Web Audio only while the track also plays through a media element, so each
  participant's raw stream also plays through a muted element (`browserMedia.ts`); without
  it the graph is silent there, while Firefox plays either way. `CallBar` above the user footer holds mute, deafen, share, and leave, and its status and place link to the call's room. Clicking a
  voice channel row joins it (a call the user is already in, or joining, is left alone) and
  opens `VoiceScreen`, the channel's screen in place of a
  history: the shared screens (one large, the others as thumbnails to pick), everyone in the
  call as tiles, a Join button when the user is not in it, and a red Leave Call button at the
  top left while they are in it or joining (`CallStage`, which a DM's call shares; nobody can
  end a call for everyone, so it only leaves). A DM or group DM holds a call too: its header's phone button starts or joins it,
  `DmCall` shows it between the header and the messages while one is under way or the user is
  in it, the DM's row carries a phone meanwhile, `CallBar` names the DM's people, and a user
  card's Call button opens the DM with that person and joins its call. No one is offered
  moderation there, since the resolver gives no one Manage calls in a DM. A call that rings
  the user (`RecordStore.myRings`, on any deployment they use) shows `IncomingCall`, a modal
  over the whole app naming who is calling and from where, with Accept (open the DM and join)
  and Decline (`AspenSync.declineCall`, as Escape does too; it has no X); while it shows,
  `startRingtone` (`src/features/notifications/ringtone.ts`, made like the chime by
  `tone.ts`) plays through the notification sound's speaker and, when the app is not focused
  and the user turned system notifications on, the system notifies, once per ring
  (`ringNotifications.ts`: a desktop may refuse a notification posted again in quick
  succession, which an effect run twice would do). A muted DM rings silently.
  A ring ends at its `until` by the clock (`useNow`). While the user is in a DM's call that
  still rings someone, `startDialTone` plays a quiet ringback (440 and 480 Hz, 1.2 seconds in
  every 4) through the voice chat's speaker. In the call, those being rung show as
  tiles darkened by `brightness-75`, at full opacity, with no visible label (a screen reader
  hears "Ringing"). A message of kind `call` renders as `CallNotice`, its length in words by
  `callLength`, and one of kind `missedCall` (a DM call no one else joined) as
  `MissedCallNotice`, "Missed call from" the caller's chip beside a red X. The large screen goes full
  screen with its corner button, a double click, or F (`ScreenTile`, `fullScreen.ts`), through
  the Fullscreen API where it works and otherwise by filling the app's window, left the same
  ways or with Escape; the mobile apps always fill the window, since Capacitor's WebView
  dismisses any element that asks for the whole screen, and there the system's back gesture
  leaves it too. While full screen, the phone turns to the picture's orientation
  (`@capacitor/screen-orientation`, `useOrientationLock`) and is freed again after; a desktop
  refuses the lock and nothing changes. On a narrow screen the call bar shows under the voice channel from the moment
  the user presses Join, so joining and a failure to join are visible there. `ChannelHeader` is the bar both
  channel screens share. Moderation lives in that same menu: server mute or unmute (`AspenSync.muteVoiceParticipant`) and remove
  (`kickVoiceParticipant`), both `202 Accepted` calls whose effect arrives as the participant's
  own events. A `participantState` frame about the user themself overwrites `muted` and
  `deafened` in the call state, which is how a server mute shows on their own controls; a
  `kicked` frame with reason `kicked` sets `endedReason: "kicked"` for `VoiceEndedDialog`,
  `replaced` (another of their own clients took over) ends the call silently, and
  `serverStopping` rejoins.
- Game capture is the desktop shell's own way to share, through libobs, which reaches games
  that display capture cannot (`game_capture` hooks Direct3D and OpenGL on Windows). The
  helper `packages/desktop/native/obs-capture` (a Rust crate, its own Cargo workspace, built
  by `pnpm build:native` in `packages/desktop`, which can run while a shell is open on Linux
  and macOS) links libobs on Windows and macOS (`src/obs.rs`, behind `cfg(obs)`), and on
  Linux only when built with its `libobs` feature, for developing the picture path there; a
  Linux build otherwise needs PipeWire's and libopus's development files and no libobs. On
  Windows the libobs is the build's own: `pnpm fetch:libobs` (`scripts/fetch-libobs.mjs`) lays
  the OBS Project's release, pinned by version and SHA-256, out in `native/libobs` (gitignored)
  with its files exactly as the OBS Project signed them, since anti-cheat systems whitelist the
  injected game hook by that signature: `obs.dll` and the libraries it and the modules need,
  the five modules the helper loads, their data with the hook's files, headers from the same
  tag's source, and `obs.lib` written from `obs.dll`'s exports (MSVC's `lib` or LLVM's
  `llvm-dlltool`), where the helper's build script finds them; the installer ships that
  directory as the `libobs` resource (`electron-builder.yml`), and the shell starts the helper
  with its libraries on the `PATH` and names its modules and data in every request
  (`bundledLibobs` in `gameCapture.ts`). CI packages the Windows installer that way. Where
  it links libobs otherwise it needs libobs's development files, which on Linux `pkg-config`
  finds and on macOS `LIBOBS_INCLUDE_DIR` and `LIBOBS_LIB_DIR` name: on macOS the `libobs`
  directory of an OBS Studio source checkout at the installed version, with an `obsconfig.h`
  written from its `.in` (OBS.app's `Contents/PlugIns` and `Contents/Resources/data` as the
  paths), a directory holding a `libobs.dylib` link to OBS.app's
  `Contents/Frameworks/libobs.framework/Versions/A/libobs` of the same architecture as the
  helper (an Apple Silicon OBS for an arm64 build; the Intel build, a bare `libobs.0.dylib`
  under Rosetta, links to nothing), the helper carrying that Frameworks directory as its rpath,
  and `BINDGEN_EXTRA_CLANG_ARGS=-I/opt/homebrew/include` for `simde` (`brew install simde`),
  which the headers include on ARM; the helper then expects OBS at `/Applications/OBS.app` at
  run time) captures one source, encodes
  it as H.264 constrained baseline, and sends it as SRTP straight to the voice server's plain
  RTP transport, answering the server's RTCP itself. With it goes the captured window's own
  sound: Windows through `wasapi_process_output_capture` and macOS through `sck_audio_capture`
  (both beta OBS features), each pointed at the same window as the picture. A game capture
  whose hook delivers no frame within `HOOK_TIMEOUT` (eight seconds: the game never presented,
  an anti-cheat refused the hook, or the window is not a game) is replaced in the scene by a
  window capture of the same window through Windows.Graphics.Capture (`window_capture` with
  the Windows 10 method, `libobs-winrt`), logged on stderr, the sound unchanged. The audio is
  encoded as Opus by obs-ffmpeg and sent as its own SRTP stream to a second producer
  (`produceRtp` with source `screenAudio`), which the sharer does not consume back. The
  helper's RTCP answers: NACKs from a buffer of recent packets, the receiver's bandwidth
  estimate by re-tuning the encoder's bitrate, and keyframe requests by relying on a
  one-second keyframe interval, since libobs cannot be asked for one. The video never passes
  through the shell or the browser, so it is encoded once. It is a separate executable, not a
  Node addon, because x264's aligned allocations trip Chromium's allocator in every Electron
  process and because native capture code must not be able to take the app down; its request
  protocol is documented at the top of its `main.rs`. `packages/desktop/src/main/gameCapture.ts`
  spawns it on first use; the preload exposes it as `window.aspenDesktop.gameCapture`. In the
  renderer, `src/features/voice/gameCapture.ts` wraps it as an `ExternalShare` for
  `VoiceCall.startExternalScreenShare`, which asks the voice server for the producers
  (`produceRtp`, answered by `rtpProduced`), hands the targets to the helper, and shows the
  preview the server sends back as a consumer of the call's own producer.
- Linux has no game hook, and its window and screen capture goes through the desktop portal,
  whose picker only the process owning the requesting window can raise, which the helper is
  not. So on Linux, X11 and Wayland alike, a game is shared as a browser screen share (the
  picture, picked in the system's picker) whose sound comes from the helper instead of the
  browser, marked `contentHint: "motion"`: `VoiceCall.startScreenShare({ audio })` takes an `ExternalAudio`, asks the voice
  server for a `screenAudio` RTP producer, and hands its target to it, dropping any sound the
  browser captured. The helper's `startAudio` request captures that sound alone through its
  PipeWire capture (`native/obs-capture/src/pipewire_audio.rs`, which the catalogue names
  `aspen_pipewire_app_audio`): it opens a capture stream and links one application's output
  ports to it, following that application by process id while it runs and by name across a
  restart, so the game keeps playing to its own output while the call hears it, and
  `app_audio.rs` cuts what arrives into 20 ms frames, encodes them as Opus with libopus, and
  sends them as SRTP. The helper advertises no video kinds there (its catalogue says
  `pictures: false`, so the shell offers no test pattern either), and lists the applications
  playing sound as `applicationAudio`.
- `GameCaptureDialog` (opened from `ShareControl`, the share button the call bar and the
  voice channel header both use) follows the platform. On Windows and macOS it lists the
  windows the capture source can be pointed at, with a checkbox for their sound. On Linux it
  lists the applications playing sound as a "Game audio" picker (preselecting the only one when
  just one plays, until the user picks), listing them again twice a second while it is open so
  an application that starts or stops playing appears or goes (a pick stands while its
  application is listed and falls to no audio when it goes), and its button opens the system's picker for the picture. Choosing the sound
  and the picture separately is a known weakness, not a design goal: one choice of "this game"
  is the better experience, and it is out of reach only because the portal tells the app
  nothing about the application behind the window the user picked. Look for ways to close
  that gap: a video track's label names the window on X11, which could identify its
  application, and a future portal may report the window's application. A development shell
  started with `ASPEN_TEST_MEDIA` naming a clip also offers it as a test pattern, looped
  through libobs's media source with its sound; without it no test pattern is offered. A shell without the
  helper on disk shows no game option. The drives run the shell headlessly with
  `ASPEN_DESKTOP_HIDDEN=1`, `ASPEN_DESKTOP_FAKE_MEDIA=1` (with which Chromium answers
  `getDisplayMedia` itself with a synthetic screen, so no picker is shown), and
  `ASPEN_DESKTOP_USER_DATA` pointing at a scratch profile.
- Screen sharing on the desktop shell always goes through a picker. Where the platform has
  one it is used: the desktop portal on Wayland and the system picker on macOS 15 and later.
  Everywhere else the main process lists every screen and window with a thumbnail and the
  renderer shows `SourcePickerDialog` (mounted in `RootLayout`), which answers over the
  `displayPicker` bridge with the chosen source and, on Windows, whether to take the system's
  audio along; dismissing it answers with nothing and the share does not start.
- Preferences are `PreferenceStore` in `packages/protocol/src/preferences.ts`, reached as
  `AspenSync.preferences` and read in components with `usePreference(definition)`. Each
  preference is a `PreferenceDefinition` with a namespaced key, a scope, a fallback, and a
  `parse` that turns whatever was stored into the value or `undefined` (so a stale entry falls
  back rather than surprising a reader). `device` scope lives in the install's storage
  (`localStorage` in a page; the sync options can supply another) and never leaves it;
  `account` scope lives on the server (`/users/@me/preferences`), is loaded at bootstrap,
  written as a merge patch, and re-fetched when a `userPreferencesChanged` event names the
  user. `get` returns the same object for the same stored value, as an external-store snapshot
  must. The audio preferences are device-scoped: `AUDIO_INPUT` and `AUDIO_OUTPUT` (the system
  default or a `NamedDevice`, id and label both, since browsers salt device ids per site and
  they can change; `resolveDevice` finds the device by id, then by label) and
  `NOTIFICATION_OUTPUT`, which defaults to following the voice output and is resolved by
  `notificationOutputDevice` for whatever plays notification sounds. `AspenSync` feeds the
  audio choices to `VoiceCall.setAudioDevices`, which swaps the microphone producer's track
  mid-call and re-routes playback with `setSinkId` where the browser has it. `SettingsDialog`
  (`src/features/settings`, the gear in the user footer) is where preferences are edited; its
  device lists come from `useAudioDevices`, which asks for the microphone once so devices are
  named and follows `devicechange`.
- Threads are channels of type `thread`, so a thread's history is a window like any channel's
  and its replies arrive as ordinary message events. A message that started one names it in
  `thread`, and the thread record carries `replyCount` and `lastReplyAt`, which
  `MessageItem` shows as a summary under the starter; message reads sideload `threads` for
  those summaries. "Reply in thread" calls `AspenSync.openThread`, which the server answers with
  the thread it makes the first time, and routes to `.../threads/{thread}`, where
  `ChannelScreen` shows `ThreadPanel` (`src/features/threads`) beside the channel, in place of
  it on small screens: the starter, the replies, and a `Composer` whose `echoTarget` offers to
  also show the reply in the parent channel (`echoToParent`). Messages in a thread cannot start
  threads. An echo is a message of kind `threadEcho` naming the reply in `echoOf`; it is drawn
  from the reply record itself (sideloaded with `echoes`, or fetched with
  `AspenSync.loadMessage`), so the reply's edits show in it. `RecordStore.channels` leaves
  threads out of their community's list though they record its id. A thread's own link, like
  any channel link that names one, redirects to it open beside its parent.
- DMs and group DMs are channels of type `dm` and `groupDm` with no community and their people
  in `recipients`. `AspenSync` reads them at bootstrap (`GET /users/@me/dms`, with their
  people) into `RecordStore.dms()` (topic `dms`): the server's order, most recently active
  first, with any DM that sees a message or is made afterwards moved to the top. A DM whose
  update no longer lists the caller is one they left, and the store drops it with its history;
  `channelRemoved(id)` then tells a screen still showing it that it is gone, not unread, so it
  is not fetched again. `/dms` is `DmLayout` (`src/features/dms`): the DM list beside the
  route's content, with "New message" opening `PeoplePicker`, which lists `RecordStore.people()`
  (everyone in the caller's communities' member lists, since a DM needs a shared community).
  A profile card's Message button opens the one-to-one DM (`AspenSync.openDm`). `DmHeader`
  titles a DM with the other people's names, each a button that opens that person's card, and,
  for a group, offers `addDmRecipient` and `leaveDm`. The list row of the one-to-one DM already
  open is a button to the other person's card, beside the row, rather than a link to where the
  reader already is; either way an unwanted conversation is a press away from a block. `ChannelScreen` serves DMs and community channels alike.
- Unread is `RecordStore` state from the `readStates` sideload of the community list and the
  DM list, one `ReadState` per channel (topic `read:<channelId>`; `useReadState`, `useUnread`),
  kept current by events: someone else's new message moves `lastMessage`, the caller's own
  moves `lastRead`, `channelRead` brings another device's reading, and a deleted message that
  was a channel's `lastMessage` makes `AspenSync` read that channel's state again. A channel is
  unread while `lastMessage` sorts after `lastRead`. `unreadPlaces()` (topic `unread`,
  `useUnreadPlaces`) names the communities with something unread, and `UNREAD_DMS` for the DMs,
  for the rail's dots, which sit half under the entry's icon. An unread channel's icon and name,
  or a whole unread DM row, are marked with `unreadMarkClass` (an accent outline over a faint
  accent fill, padded so nothing moves), and every unread row's accessible name says so. `MessageList` marks the newest message on screen read through
  `AspenSync.markRead`, which updates the store at once and reports to the server at most every
  `READ_REPORT_MS` per channel, and only while the page is visible and focused; leaving the
  channel or hiding the page flushes the report. The "New Messages" line is placed where the
  read position was when the channel opened, and only if it was unread then, and stays there
  while the channel is open; posting removes it.
- Muting is store state from the `mutes` sideload of the community and DM lists (replaced
  whole at each bootstrap by `replaceMutes`), kept current by `channelMuteChanged` events;
  `AspenSync` ends timed mutes by the device's clock (`nextMuteEnd`, `expireMutes`), since the
  server announces no end it did not make. A muted channel or DM (`useMute`) is drawn in
  `text-ink-faint` with a muted bell, is never marked unread, and does not count toward
  `unreadPlaces`; its read position is kept, so it is unread again once the mute ends, and the
  "New Messages" line still shows inside it. `ChannelMenu` (`src/features/channels`) holds two
  submenus, Mute (one of the offered lengths, or, while muted, until when and Unmute) and
  Notifications (naming the level in force), above the channel's other actions; it opens on a
  right click on a text channel's or DM's row, or from the row's `ChannelMenuButton`, which
  keyboards and touch screens use, since a long press on a row starts dragging it. It opens
  beside the row, and below it on a one-pane screen, where there is no room beside it.
- Tags (`<@user>`, `<@&role>`, `@everyone`) are drawn by `remarkMentions` and `Mention`
  (`src/features/messages`): a tag the message's `mentions` says counts is a chip, a person's
  opening their card; any other is plain text naming whom it would have tagged, as the server
  leaves it. A message that tags the reader (`RecordStore.mentionsMe`: by name, a role they
  hold, or everyone) is marked (`data-mentions-me`). In a message box, `useTagging`
  (`src/features/mentions`) offers people, roles, and everyone as `@` is typed, each only with
  its permission in the channel (people from the member sample and, for those the server lets
  search a large community, `useMemberSearch`), from the keyboard (arrows, Enter or Tab, Escape) with
  `aria-activedescendant` and a polite status, since React Aria's ComboBox cannot complete at a
  `TextArea`'s caret. Completions share `SuggestionList`, and the box renders one status saying
  what the active completion's `announcement` says. A pick shows as `@username` or `@Role` and is sent as its tag
  (`encodeTags`); editing reads tags back (`decodeTags`). Each read state's `mentions` is the
  unread tags of the reader: the store adds a live message that tags them, clears it when they
  post or read to the newest, and `AspenSync` reads the state again when only the server can
  say (reading partway, another device reading, an unread message's tags edited or it
  deleted). `MentionBadge` shows the count on channel and DM rows and, summed
  (`placeMentions`), on the rail, muted channels included, each row saying it in its
  accessible name. `e2e/tagging.spec.ts` shows a screen reader user tagging from the keyboard
  alone and checks the box and its suggestions with axe (`@axe-core/playwright`).
- Custom emoji are a community's own (`CustomEmoji` records, from the `emoji` sideload of the
  community reads and `customEmoji` events; `RecordStore.customEmoji(communityId)`, topic
  `emoji:<communityId>`, `useCustomEmoji`), each a name and a picture that is an icon. A
  message's text names one as `<:id>` (`src/features/emoji/customEmoji.ts`, which
  `remarkCustomEmoji` turns into `CustomEmojiGlyph`: the picture at the text's size, named
  `:name:` for assistive technology, or a marked placeholder for an id the community's list
  does not hold); the message box shows `:name:` and `useEmojiCompletion` (`src/features/emoji`)
  completes a colon word with the community's emoji and every unicode emoji by its names (the
  picker's own data in the reader's language, `emojiData.ts`: the catalogue they chose, or for
  `automatic` the first of the browser's languages the picker has, one file loaded on first
  use; the picker takes the same file through `emojiData`, its custom section renamed to the
  app's words), a unicode pick writing the glyph and a custom pick `:name:`, which `encodeCustomEmoji` turns into the reference as the message is sent and
  `decodeCustomEmoji` back for an edit. A reaction's key may be the same reference, so
  `ReactionChips`, the reactions dialog, and the picker (`customEmojis`, with the pictures from
  `useIcons`) take the community, and a DM, which has none, passes `null`. The store drops the
  reactions made with an emoji when its deletion event comes, since the server's cascade
  announces them no further. Community settings' Emoji tab (`EmojiPanel`) lists them and, with
  Manage custom emoji, adds one (`prepareEmojiPicture` scales a still picture to 128 pixels and
  refuses a larger GIF), renames, and removes.
- Bots are users with `bot` set, marked by `BotBadge` wherever they are named (messages, the
  member list, their card, which also says who made them, the admin users directory). Nothing
  in the app signs in as a bot; bots use the API with their tokens. Developer mode is the
  account preference `DEVELOPER_MODE`, turned on in Settings. With it comes the ID wizard
  (`ID_WIZARD`, on unless turned off; `useIdWizard`): everything with an id offers to copy it
  through `CopyIdButton`, or `CopyIdMenuItem` in a menu (`src/features/layout/CopyId.tsx`),
  always last in whatever shows it, so the everyday controls keep their places whether it is
  on or off, and each dashboard table gains a last ID column (`Directory`'s `idThing`). A new
  place that shows something with an id gives it one. Developer mode also offers `BotsDialog`
  (`src/features/bots`): the caller's bots (`RecordStore.ownedBots`, topic `bots`, derived from
  the cached users whose `botOwner` is the caller and filled by `AspenSync.loadBots`), making
  one, whose token is shown once as it arrives, its picture (`IconPicker`, written with
  `AspenSync.updateBotProfile`), public or private, the link that adds it with
  the permissions it suggests (`botAddLink`: the page's address on the web, the path in the
  shells), a new token, handing it to someone (`PeoplePicker`), and deleting it. The writes are
  `AspenSync.createBot`, `rotateBotToken`, `setBotPublic`, `transferBot`, and `deleteBot`,
  applied from their answers, since a bot's own events reach only those who share a community
  with it. The link opens `BotAddScreen` (`/bots/$botId/add?permissions=…`), which offers the
  communities where the caller holds `addBots` and the suggested permissions the caller may
  give (Manage roles and Assign roles, and each permission held), and calls `AspenSync.addBot`.
  A role made for a bot says whose it is in the role editor and is offered for neither
  deletion nor assigning. With `manageBots`, the admin users directory deletes ownerless bots.
- The system account (`User.system`, the deployment's own, which sends notices) is marked by
  `SystemBadge` beside `BotBadge`, and its card offers no way to message, call, or block it.
  Its DM is read-only: `RecordStore.channelAccess` gives only View channel there, as in a
  blocked DM (`systemDmPeer`, topic `channelAccess:<channelId>`, touched when the account's
  record arrives), and the Composer shows a note in place of the box.
- Motion. Every animation and transition takes its duration from the tokens in `styles.css`
  (`--motion-fast`, `--motion-base`, `--motion-slow`, `--motion-highlight`), which scale with
  the account preference `MOTION_SPEED` (`look.motionSpeed`, 0 for off to 2 for twice as fast,
  set by `MotionSpeedSlider` in Settings, Appearance): `useFollowMotionSpeed` writes
  `--motion-scale` on `<html>`, and `data-motion="off"` there stops every animation and
  transition but those marked `data-motion-essential`. Tailwind's `transition-*` utilities take
  the scaled duration too. At normal speed most take 200ms or less (`--motion-base`), a panel's slide 270ms, and a highlight 1.6s. A
  device set to reduce motion keeps fades and drops travel and growth (the distance and scale
  tokens go to nothing). The utilities are `motion-backdrop` and `motion-dialog` for modals
  (every modal has them through `overlayClass` and the modal classes), `motion-rise`,
  `motion-drop`, `motion-fade`, `motion-grow`, and `motion-from-end` for things that appear,
  `motion-pop` for something that changed, and `motion-flash` for a message gone to; every popover, menu, select list,
  and tooltip moves by one rule on `[data-placement]`, so a new one needs nothing. What moves:
  modals and overlays; messages that arrive while shown (`RecordStore.arrivedAt`), never
  history; the space of a message deleted while shown closing (`useDeparting`,
  `RecordStore.departedAt`, and `DepartingSpace`, which starts closing on the first frame after
  the render that made it, so a slow render does not use the animation up unseen); a message gone to flashing; reaction chips and mention badges
  popping as they grow (`useGrowthKey`); channels revealed by unfolding a category and
  communities of a folder opened here dropping in; rows gliding to new places when an order
  changes (`useReorderGlide`); the thread panel and member list sliding in; call tiles, the
  composer's files (with an upload progress bar, from `uploadAttachment`'s `onProgress`), the
  sync banner, and the Jump to latest pill appearing; speaking rings fading. Code that moves
  things itself reads `useMotion` (off, reduced, scale).
- The channel list (and the DM list in its place), the member list, and the thread panel are
  `ResizablePane`s (`src/features/layout`): each has an edge, a `separator`, that the reader
  drags, moves with the arrow keys (Shift for bigger steps, Home and End for the bounds), or
  double-clicks (or presses Enter on) to reset. The width is a device preference per pane
  (`paneSizes.ts`: `layout.<pane>.width`), kept when a drag ends and read within the pane's
  bounds. Each is kept with its pane's layout version, and a width kept under another version
  is ignored, so an update that changes a pane enough to need it raises that pane's version and
  resets it alone; otherwise widths are left as the reader made them. On one-pane screens a
  pane fills the screen and has no edge.
- Every list sidebar, the channel list's and the DM list's, is drawn by the same two pieces:
  `SidebarHeader` (its title and plain icon buttons, `headerIconButtonClass`, spaced so their
  touch areas never overlap) and `SidebarFooter` (the call bar and the user bar).
- The message box (`Composer`) sits level with its buttons, all 42px tall; Send is an icon
  (paper plane), so the box keeps the room. Enter sends on a computer; on a touch-only device
  (`TOUCH_ONLY`), whose keyboard has no Shift+Enter, Enter writes a new line there and in a
  message being edited, and the button sends. On a narrow screen
  its other controls share one + button whose menu offers attaching a file and making a poll
  (`CreatePollModal`, which the wide screen's own poll button opens too). Its placeholder is
  drawn by the box itself on one line, cut short with an ellipsis, since the textarea's own
  would wrap and grow the box; the textarea carries it as `aria-placeholder`. When the list
  shrinks, as when a phone's keyboard opens, it keeps its bottom edge where it was (pinned to the
  newest message, or moved down by what it lost), so what is being answered stays in view; the
  web app asks for this with `interactive-widget=resizes-content` in its viewport. "Jump to
  latest" answers at once: the pill keeps its own state, so a press repaints it alone, saying
  the newest are on their way, before the list moves to the end of what it holds, and `AspenSync.loadLatest` follows any page already being read rather than
  settling for it.
- Drafts (`src/features/messages/drafts.ts`): what is written in a message box and not sent
  (text, picked tags, files already uploaded, and a thread's echo choice) waits in its channel
  on this device, per account, through going elsewhere, a notification opened, and a reload,
  and goes once sent. The Composer is keyed by its channel, so each box reads its own; it notes
  its draft in memory on every change (`noteDraft`), which `readDraft` reads first, and keeps it
  in `localStorage` after a short pause and at once when it unmounts or the page is hidden.
  `e2e/drafts.spec.ts` checks the notification path.
- Bots' commands (`src/features/commands`) are offered in the message box by `useCommandLine`,
  which the Composer runs beside `useTagging` and which turns tagging off while the draft
  begins with `/`. The channel's commands are store state (`RecordStore.commands`, topic
  `commands:<channelId>`, read by `AspenSync.loadCommands` through `useCommands`), dropped
  whenever something may change which bots see the channel or what they answer
  (`botCommandsChanged`, roles, overrides, a bot's membership, a DM's recipients), and read
  again where shown. Typing the name offers every bot's commands with the bot's picture and
  name and the command's description in the reader's language (`describe`); picking one that
  another bot answers too writes its bot's tag after the name (`/kick @modbot `), which is how
  a line says which it means. While an argument is typed, the command's form shows above the
  box with the parameter at the caret marked, what it takes, and a warning when a `regex`
  value does not match; people, roles, channels, communities, and deployments are suggested by
  name, and a pick is remembered so that sending turns its text into the id. `commandLine.ts`
  holds the reading, the server's quoting included (`tokenize`, `quoteArgument`), the last
  `any` parameter taking the rest of the line, files filling `attachmentId` parameters in
  order. Sending a line that names a command here invokes it (`AspenSync.invokeCommand`), one
  two bots answer untagged is refused in the box naming both, and any other line beginning
  with `/` goes as a message. A command's message says who it was sent to beside its author
  (`sentTo`), and its text names people and roles by the tags that tag no one, so they read by
  name.
- Blocks are store state (`RecordStore.blocked`, topic `block:<userId>`, and `blockedUsers`,
  topic `blocks`; `useBlocked`, `useBlockedUsers`), read at bootstrap from `GET
/users/@me/blocks` with the blocked users sideloaded and kept by `userBlockChanged` events.
  `AspenSync.blockUser` and `unblockUser` apply their answer at once; a change from either
  source sets the user's call gain (`#userGain`: silent while blocked, whatever their volume
  and "mute for me" say) and reads again what the server counts without them: every read
  state, and the reactions of each held message window, one `around` read per window. The
  stream is filtered the same way, so a blocked user's reactions are never counted and their
  messages never move `lastMessage`. `MessageList` collapses each run of consecutive messages
  by blocked people into one `BlockedRun` row ("2 blocked messages", Show/Hide), which stands
  for the run's last message when anchoring the view and marking it read; a linked message
  inside a run opens it, and a thread's starter collapses the same way. A pin by someone
  blocked is not quoted. In a one-to-one DM with someone the caller blocked the resolver
  grants only `viewChannel` (`blockedDmPeer`), so every write control goes and the composer
  gives way to a note offering to unblock; a block the other person made is known only from
  the server's `blocked` refusal. In calls a blocked person is marked on their tile and row,
  their screens are left out of `VoiceScreen`, and their menu offers no volume. The profile
  card blocks (after a confirming second press that says what blocking does) and unblocks,
  the member list marks blocked people, and Settings lists them with Unblock.
- Folded categories are store state from the `collapses` sideload of the community list
  (replaced whole at each bootstrap by `replaceCollapsed`), kept current by
  `categoryCollapseChanged` events (`useCollapsed`). A category's heading in `ChannelSidebar`
  is a button that folds it (`AspenSync.setCategoryCollapsed`, `aria-expanded`); folded, it
  still lists the channel being viewed and whatever `RecordStore.shownWhenCollapsed` keeps (an
  unread, unmuted channel, or a voice channel with someone in the call; `useShownWhenCollapsed`
  follows them). Drops and reorders in a folded category still place channels among all of
  its channels, shown or not.
- Reactions are store state per message (`RecordStore.reactions`, topic `reactions:<messageId>`):
  per emoji, the count, whether the caller reacted, and the first few to react, installed from
  the `reactions` sideload of every message read (`setReactions`) and kept current by `react`
  events. The caller's own reaction is applied from the request's answer and again from its
  event, which changes nothing the second time. When one of the named few leaves, `AspenSync`
  reads the message's summary again. `ReactionChips` (`src/features/messages/Reactions.tsx`)
  shows the most popular first (ties in the order first used), at most twenty, then a `+N` chip
  and an add chip that opens the picker at once; each chip's tooltip names the first four and
  counts the rest; emoji are drawn half again as large as the count, and the reader's own
  reaction is marked by a darker outline in the chips' neutral colours. `ReactionsDialog`,
  opened from the `+N` chip, the message's "View reactions" action, or a right click or long
  press on a chip (opening on that chip's emoji; the long press toggles nothing), lists every emoji and, for the chosen one, everyone who reacted, a page at
  a time (`AspenSync.loadReactors`).
- The Administration Dashboard is `/admin/{tab}` (`src/features/admin`; `/admin` opens the first
  tab the caller may), a rail of tabs beside the one open, set across the top on a one-pane
  screen, each section on it a plane; it is offered in the community
  rail to anyone with a deployment permission (`RecordStore.deploymentPermissions`, topic
  `admin`, read at bootstrap from `GET /users/@me/admin` and kept by `deploymentAccessChanged`
  events; `useDeploymentPermissions`, `useDeploymentCan`, `useIsAdmin`). Each tab shows
  only with the permission it needs: the totals and growth, and the fleet, with `viewDashboard`,
  registration invites with `manageRegistrationInvites`, the directories with `viewDashboard`
  or `moderateCommunities`, the moderation log with `viewDashboard`, federation with
  `manageFederation` (`Federation.tsx`: this deployment's domain, key fingerprint, and gates;
  adding a deployment, which is contacted at once; and the directory of those known, each
  checked again, put on the lists the gates read, its offered key reviewed and accepted, or
  forgotten), and the deployment roles to everyone, editable with `manageDeploymentRoles` below
  the caller's highest role (`deploymentRoleRecords.ts`). The directories share `Directory`, which
  reads its page again when its `version` changes. The users directory shows each person's deployment roles and, for
  those who may, a picker to change them; for a moderator, it lists anyone's DMs to open, and
  the communities directory opens any community. The moderation log names what each entry names from the server's `details`: people as chips that open their cards, communities, channels, DMs, and messages as links while they stand, deleted ones by name, and only what has no name as its id. A moderator's access is part of the resolver
  (`CommunityPermissions.moderator`, `MODERATION`, both in the shared vectors): they see every
  channel, may take things away (delete messages, remove attachments and others' reactions,
  remove members, rename and delete channels and communities), and are told in the sidebar
  when they are in a community only as a moderator. They read no events of a community or DM
  they are not in, so `AspenSync` applies their writes there to the cache itself
  (`#readsEventsOf`). Their reads are the server's answer of the moment rather than cached
  records, held where they are shown by `useAdminRead`, which also re-reads the fleet every ten
  seconds while the page is visible. Figures follow the dataviz rules: stat tiles for totals,
  tables with aligned figures for the fleet, and every state an icon and a word, never color
  alone. Growth is two single-series line charts (`LineChart`, hand-drawn SVG) under one row of
  range buttons, users and communities apart because one chart with two scales would invent a
  relation between them; each has a 2px line over a 10% wash, round-number gridlines from zero,
  its latest value at the end, a crosshair and readout on hover and from the arrow keys, and the
  table view carries every value. The user and community lists sort from their headings
  (`aria-sort`) and page by offset, 15 rows by default, with the page size chosen beside them. A
  registration invite copies as `/register?invite=CODE` where the app has a web address, or as
  its code in the shells. That link opens the create-account screen with the code filled in,
  and `RegisterForm` asks for a code whenever `GET /auth/methods` says the server requires one
  (`useAuthMethods`).
- Roles and permissions resolve on the client with `packages/protocol/src/permissions.ts`, the
  same rules as the server's `app::permissions`; both run the cases in
  `spec/permission_vectors.json` at the repository root, so a change to either resolver must
  change the vectors and the other resolver with it. The store holds each community's roles,
  channel and category overrides, and every known member's roles (from the `roles` sideload,
  which the bootstrap and `loadCommunity` ask for, and from `role`, `channelOverride`,
  `categoryOverride`, and `userCommunity` events), and answers `access(communityId)` (topic
  `access:<id>`, hook `useAccess` / `useCan`) and `channelAccess(channelId)` (topic
  `channelAccess:<id>`, `useChannelAccess` / `useChannelCan`; a thread's are its parent's, a
  DM's every channel permission and pinning). Controls the caller may not use are not shown:
  adding and arranging channels, the invite manager's create and revoke, the composer's
  attach, poll, and send (a note replaces the box without sending), message delete, pin,
  reaction, and thread actions, voice moderation, joining, and sharing; renaming and deleting a
  channel from its menu, and removing an attachment or someone's reaction. The server refuses
  anyway; hiding only keeps the UI honest. When the caller loses sight of a channel the store
  lets it go at once (as `channelRemoved`); when they may gain some, `AspenSync` reads the
  community again after a random pause of at most `ACCESS_RELOAD_SPREAD_MS`, since the server
  sends nothing about channels a member could not see. The same holds for invites: without
  Manage invites the server sends only the caller's own, and the store lets the rest go when
  the permission is lost. `UserCommunity.sortIndex` is `null` on everyone's membership but the
  caller's. A join offer says whether the caller
  may speak and share (`VoiceCallState.canSpeak`, `canShare`); without Speak the call joins to
  listen, opening no microphone. `src/features/community-settings` is the management UI: the
  sidebar gear's `CommunitySettingsDialog` (name and icon, ownership, delete or leave; roles,
  with templates and a grouped `PermissionChecklist` whose unheld permissions are disabled; and
  members, with their roles, Remove, and, for holders of Ban members, Ban: `BanDialog` takes a
  reason, how long for, and, for a banner who also holds Manage messages, whether their messages
  from the last hour or day go too, and `BannedList` beneath the members shows the standing
  bans (`RecordStore.bans`, topic `bans:<communityId>`, read on first use by `useBans` and kept
  by `communityBan` events, which reach holders of Ban members) with Lift ban; someone banned
  who tries an invite is answered `banned` with the reason, which the invite screen shows), and
  `AccessDialog`, opened from a channel's menu or a
  category's lock, which leads with three presets (everyone, only some roles, read-only) and
  keeps per-role allow, default, and deny under Advanced and a per-member explanation
  (`explain`) under Check access. Wherever a member is chosen (the Members tab, handing over
  ownership, Check access), `useMemberSearch` shows the member sample until something is typed,
  and then, for those the server lets search every member (the same rule, mirrored so the field
  is not offered to anyone refused), searches with `AspenSync.searchMembers`, which caches the
  people found and their roles without adding them to the sample; `MemberPicker` is the
  combobox the dialogs share. A preset lets its roles in before shutting everyone else out,
  so their members never lose the channel in between. Pins are store state per channel (topic
  `pins:<channelId>`, `usePins`, read once on first use and kept by `pin` events), shown by
  `PinsButton` in the channel and DM headers, each pin with its author's picture and drawn by
  `MessageBody`, the same body the channel draws (text, tags, pictures, files, poll, link
  cards), with Unpin for those who may pin.
- Presence is pulled. The server pushes no status events; `AspenSync` asks
  `GET /users/statuses` for `RecordStore.presenceCandidates()` (the members shown for every
  community and everyone in a call), in batches of `PRESENCE_BATCH`, when the sync goes live,
  every `PRESENCE_POLL_MS` while the page is visible, and when the page becomes visible again;
  `applyStatuses` is the only way a user's `onlineStatus` changes after the bootstrap.
  What makes the server show the user as online rather than away is `AspenSync.noteActivity`,
  which `SyncProvider` calls on pointer, keyboard, wheel, and touch input and when the window
  gains focus or comes into view (`src/api/activity.ts`); it sends an `activity` frame at most
  every `ACTIVITY_INTERVAL_MS`, and on reconnecting only if the user was active within that
  interval. Nothing else may call it: background work is not the user using the app.
- Events are routed by the server to the communities the user belongs to and to the user alone,
  leaving out channels they may not view, so the client filters nothing itself. An event about a
  user reaches the client once per community shared with them; every copy carries the same
  `eventId` on its frame and `EventStream` drops all but the first of the last few thousand ids it
  has seen.
- Update events and update requests are JSON Merge Patches: an absent field is unchanged, `null`
  clears a nullable field. Apply them field by field; never replace a cached record wholesale
  with an update payload.

## UI

- Whatever waits on the network stands in skeleton meanwhile (`Skeleton` and `LoadingLabel`,
  `src/features/layout/Skeleton.tsx`, and the shaped ones in `ScreenSkeletons.tsx` and
  `MessageSkeleton.tsx`), sized like what it stands for, or a conservative guess where that
  cannot be known, so nothing moves when it arrives; it pulses gently, still where the reader
  asks for less motion, is hidden from assistive technology, and the region says it is busy
  and names what it waits for. A record fetched on demand is waited on only until the server
  says it does not exist (`RecordStore.missing`, `markMissing`, on the topic its record would
  come on; `useUserLoading`, `useIconLoading`, `usePollLoading`), so a skeleton never stands
  forever and "unknown" or "unavailable" never flashes before the record arrives; people are
  named through `PersonName` and `PersonAvatar` for that. A picture keeps its room while it
  loads: an uploader measures it (`measurePicture`) and the attachment records its width and
  height, a link preview's picture carries the size the server read from its header
  (`imageWidth`, `imageHeight`), and a picture without either keeps a conservative box.
- Components come from `react-aria-components`. Do not reach for `react-aria` hooks or another
  component library unless React Aria genuinely lacks the primitive; if so, say why in a comment.
- Every screen and dialog passes axe (`@axe-core/playwright`): `e2e/accessibility.spec.ts`
  audits them in every palette and both colour schemes on Chromium, and in the default palette
  on the other browsers and phones. So every ink token, `ink-faint` included, reaches 4.5:1 on
  each surface text sits on; a new palette or token must too. Content sits in landmarks: each
  sidebar is a `section` named by its `h1`, the conversation is `main`, and where one pane
  shows at a time (`useOnePane`) the pane shown is `main` and its title the `h1`, headings
  below it moving up a level. A popover is audited without the landmark rule, since React Aria
  renders it at the end of the page, outside every landmark, as a floating layer must be. A
  region that may scroll sideways is focusable (`tabIndex={0}`) so a keyboard can scroll it.
- Colour comes only from the semantic tokens in `src/styles.css` (`bg-surface`, `text-ink-muted`,
  `border-line`, `bg-accent`, `text-danger`, ...). Do not use Tailwind's named colours or
  `dark:` variants: each token is a `light-dark()` pair inside a palette, and a palette is a
  `[data-theme]` block of variables that `src/theme/palettes.ts` switches at runtime. Which of
  each pair shows is the root's `color-scheme`: `light dark`, following the system, unless the
  user chose Light or Dark under Appearance (`applyThemeMode`, kept per install like the
  palette). Nothing may read `prefers-color-scheme` itself, since that ignores the user's choice.
- Icon-only controls get a `Tooltip` (`src/features/layout/Tooltip.tsx`) whose text is also their
  `aria-label`, so the tooltip and the accessible name never disagree.
- Something done and done with, where nothing on the page will say so (text copied), is said in
  a toast: `toast(text)` (`src/features/layout/toast.ts`, React Aria's toast queue) from
  anywhere, shown by `Toasts` for a few seconds, dropping in over the top of the channel's
  messages (`ChannelScreen`, and the thread panel's own where it is the whole screen); the root
  mounts a `fallback` region fixed to the top of the page, shown only while no channel's region
  is mounted, so a toast sent from the channel list or a dialog opened there still shows, once.
- Icons come from `@phosphor-icons/react` (MIT, imported by their `…Icon` names, tree-shaken).
  Prefer an icon to an emoji glyph in controls. Emoji reactions use `emoji-picker-react` with
  native glyphs, loaded lazily by `src/features/messages/Reactions.tsx`; it fetches nothing.
- Styling is Tailwind CSS 4 with `tailwindcss-react-aria-components`, so interaction states are
  the `pressed:`, `selected:`, `focus-visible:`, `invalid:` variants driven by React Aria's data
  attributes. Do not add `:hover`/`:active` CSS by hand for those states.
- Phones get the same app, one pane at a time. Below Tailwind's `md` breakpoint a list (the
  channel list, the DM list) and a conversation never share the screen: a conversation takes the
  whole width, the community rail included, and its back link leads to the list. Layout choices
  made in code use `useMediaQuery(MEDIUM_SCREEN)` (`src/features/layout/useMediaQuery.ts`) so they
  agree with the class names; `CommunityIndex`, for one, opens the first channel only on a wide
  screen, since on a narrow one the index is the channel list.
- Copy to the clipboard with `copyText` (`src/features/layout/clipboard.ts`), never
  `navigator.clipboard` directly: the Clipboard API is missing on a page served over plain HTTP,
  such as a dev server a phone reaches by address, and `copyText` falls back to a copy that
  works there and in iOS Safari.
- Nothing may depend on hover, which a touch screen does not have. A control revealed on hover is
  also revealed by focus, or offered another way on a touch screen: a message's actions
  (`MessageActions`, icons at `ACTION_ICON`) sit at its corner on hover or focus where there is
  a pointer, and on a touch-only device (`TOUCH_ONLY`) a long press on the message opens them in
  a popover beside the finger (`useLongPress` from `react-aria`, the one hook taken from it,
  since the components package has no long press; the popover is anchored to the point
  pressed, so on a long message it comes where the finger is, below it, or above it in the
  lower half of the list and always on the newest message, which sits on the message box),
  settling into place with `motion-settle` and a tap felt in the hand in the apps
  (`haptics.ts`, `@capacitor/haptics`); the press owns the message there, so the browser's
  text selection is off on it and Copy text is among the actions. Every action closes the
  popover: the ones that open something (the reaction picker, who reacted, delete) open it
  as a sheet of `MessageItem`'s own, anchored to the same point, since a popover or dialog
  inside the actions would go with them (`MessageSheet`, `ReactionPickerPopover`,
  `ReactionsDialog`, `DeleteMessageModal`). Otherwise a control is shown outright with
  `pointer-coarse:`. Small icon controls take `tap-target` (`src/styles.css`), which
  widens what a finger can hit to 44px on a touch screen without moving anything; controls side
  by side are drawn larger with `pointer-coarse:` instead, so their areas do not overlap. The app
  pads itself by the safe-area insets, since the page is laid out under a notch
  (`viewport-fit=cover`); a modal's overlay sits outside `#root` and pads itself the same way
  (`overlay-inset`).
- Every modal is drawn in `modalClass` (or `wideModalClass`) from
  `src/features/invites/dialog.ts`, which never grows taller than the screen and scrolls within
  itself, so a long form on a phone reaches its buttons. Build new modals on those rather than
  on a class string of their own.
  A modal or card of several sections draws each as a plane raised off a plainer ground:
  `planesModalClass` or `widePlanesModalClass` for the modal, `planeClass` for a section
  (`planeSurfaceClass` for one that lays out its own contents, such as a list), and
  `PlaneColumns` to let planes stand in two columns where the modal is wide enough. Settings,
  Sign-in and security, community settings, the bots dialog, the user card, and the
  Administration Dashboard's sections are drawn this way.
  Every modal is titled with `DialogHeading` (`src/features/layout/DialogHeading.tsx`), which
  puts a Phosphor X in its top right that closes it through the `Dialog`'s `close` slot, and
  has no other button that only closes it (no Cancel, Close, Done, or OK); whatever closing
  must do (answering the screen picker, abandoning a crop) goes in the overlay's
  `onOpenChange`, which the X, Escape, and a click outside all reach. The exceptions are a
  modal that must not be left before the reader acts, such as the recovery codes, and an
  incoming call (`DialogHeading closeButton={false}`), whose Accept and Decline are the ways
  out; a click beside it does nothing, and Escape declines.
- User-facing strings live in `packages/app/src/i18n/messages.ts` with camelCase keys, matching
  the server's locale files. Server Problem text is already localized and is shown as-is.
  The language shown (`src/i18n/locales.ts`, `I18nProvider`) is the account preference
  `LANGUAGE`, which Settings sets and `FollowLanguagePreference` applies once the account's
  preferences are read, or `automatic`, which follows the browser's languages; this install
  remembers the last one for the sign-in screen. It picks the catalogue (English, or the
  pseudo-locales `en-XA` and `ar-XB` made from it by `src/i18n/pseudo.ts`), React Aria's
  locale, the document's `lang` and `dir`, and the languages every `AspenClient` request and
  event stream asks servers to write in (`setPreferredLanguages`). Dates and numbers are
  formatted in that locale through `useDateFormat` and `useNumberFormat` (`src/i18n/format.ts`)
  and the dashboard's `useFigures`, never `Intl` with the browser's default. Layout is written
  for both directions: logical utilities (`ms-`, `pe-`, `start-`, `border-s`, `text-start`),
  never `ml-`, `pr-`, `left-`, `border-l`, or `text-left`, and an icon that points somewhere
  (a back arrow, a next caret) mirrors with `rtl:-scale-x-100`; a chart of time keeps `dir="ltr"`.
- Message search (`src/features/search/SearchDialog.tsx`) opens from the channel and DM headers
  and searches the channel, its community, its server, or every deployment the user uses
  (`useSources`), each through its own `AspenSync.searchMessages`. Several deployments' pages
  merge newest first with `mergeResults`, which shows nothing older than the oldest result of
  a deployment that has more to give, so a later page never lands above what is shown. Each
  result renders in its deployment's `SourceScope`, as a plain-text preview with tags as names
  (`decodeTags`), since a result is a link and a message's Markdown may hold links of its own.
- Notifications: each user's settings (`notificationSettings`, from the community and DM reads
  and `notificationSettingChanged` events) are store state, `RecordStore.notificationLevel`
  (topic `notifications`, `useNotificationLevel`) resolving a channel's level from its own
  setting, its community's, and the default, and `notifies(message)` deciding whether a message
  tells of itself. `AspenSync.onNotify` announces such messages as they arrive live, never the
  replay a connection starts with. `NotifyOnMessages` (`src/features/notifications`), for every
  deployment the user uses, plays the chime (`chime.ts`, a two-note sound made in code, through
  the notification speaker) and, when `DESKTOP_NOTIFICATIONS` is on and the browser allows, shows
  the system's notification (`describe`), which opens the message; nothing for the conversation
  in view, and no system notification in the mobile app, whose phone push wakes. A channel's menu
  sets its level ("Notify me about"), community settings the community's
  (`CommunityNotifications`), and Settings the two device preferences (`NotificationsSection`).
- Push (`spec/push.md`): on the mobile app, `WakeThisPhone` (`src/api/push.tsx`) asks to notify,
  registers with `@capacitor/push-notifications`, and keeps a relay subscription with every
  deployment of `useSources` through `syncPush` (`packages/protocol/src/push.ts`, which also
  decrypts a push, RFC 8291, as the native code must), saving the `PushState` through the native
  plugin `AspenPush` (`src/api/pushBridge.ts`), which says which platform, app, and relay the
  build is. A build without that plugin simply has no push. Signing out forgets every account.
  Tapping a notification opens its message. On Android (`packages/mobile/android`, the one
  native project kept in git), `AspenPushPlugin` holds the state in the app's private storage,
  and `AspenMessagingService` takes the place of the push plugin's FCM service (the manifest
  removes that one), handing it anything that is not a relay push: `PushHandler` decrypts
  (`WebPush`), fetches with the account's session, refreshing it on `401`, and posts the
  notification, tagged `channel/message` so `read` and `deleted` take it down. A build pushes
  only with a relay in the `aspen_push_relay` string and a `google-services.json` from its
  publisher's Firebase project. On iOS (`packages/mobile/ios/App`), `AspenPushPlugin.swift` is the
  same plugin: `describe` names APNs, the bundle id, the sandbox for a debug build, and the relay
  `AspenPushRelay` in `Info.plist` names (empty, like Android's string, until a publisher sets
  it), and the state is a keychain item in the access group `AspenKeychainAccessGroup`
  (`$(AppIdentifierPrefix)` and the bundle id with `.push`, in both `Info.plist` files and both
  entitlements files), which the notification service extension `AspenNotificationService`
  shares (`PushState.swift`). The extension (`NotificationService.swift`) decrypts the push's
  ciphertext with the account's keys (`WebPush.swift`, CryptoKit; `ios/scripts/check_webpush.sh`
  runs it against RFC 8291's example with `swiftc`, no target needed), fetches the message
  with the session (renewed once on `401`, the new token written back), and shows the author,
  the channel, and the text with tags as names, carrying where the message is for a tap; a
  `read` or `deleted` pointer, and any failure, leaves the placeholder, since the build holds no
  filtering entitlement. The `AppDelegate` posts APNs registration to the Capacitor push plugin.
  `xcrun simctl push <device> org.aspenchat.client payload.json` delivers a push to the
  simulator without APNs or the relay, with `aspen.s` and `aspen.c` as the relay would send them. `pnpm --filter @aspen/mobile test:android` runs the JVM tests
  and, on a connected device or emulator, the handler end to end against a stand-in deployment
  (debug builds may use plain HTTP to the device itself for it). It needs JDK 21 and the Android
  SDK; CI does not run it yet.
- Two builds of the same code: `pnpm build` (web, served from a site root, real URL paths) and
  `pnpm build:shell` (`--base ./`, used by the desktop and mobile packages, which load the bundle
  from `file://` or an app-local origin and route after a `#`). Never write an absolute
  `/assets/…` URL by hand; let Vite resolve assets so both builds work.
- Routing is TanStack Router (`src/router.tsx`), code-based, one route tree for every shell.
  Anything a user might want to share is a route: `/communities/{id}/channels/{id}`,
  `.../messages/{id}`, and `.../threads/{id}`, and the same under `/dms/{id}` for DMs. Read
  params with `useParams`; navigate with `Link` and `Navigate` rather than by building URLs.
  The message components live under both trees, so they take the channel's home (a community
  id, or `null` for a DM) and build their links with `src/features/messages/links.ts`. A message link's id segment leaves the URL once the reader scrolls on
  their own (wheel, touch, scrollbar, or a navigation key, tracked in `MessageList.tsx`); scroll
  events the browser fires for layout changes, image loads, or scripted scrolling do not count,
  and dropping the segment never reloads the window. History pages in on its own, well ahead
  of the reader (`loadOlder` / `loadNewer`, `HISTORY_PAGE_SIZE` messages at a time): within
  `LOAD_AHEAD_SCREENS` of the end they are heading for, as their own wheel, finger, or keys
  last moved the list, and never at the end behind them (within `LOAD_UNSURE_SCREENS` of either
  while that is unknown): a page read at one end of a full window drops messages from the
  other, and reading at both would trade pages back and forth, changing what is above the view
  each time. Nor is a page read whose arrival would drop messages within `LOAD_UNSURE_SCREENS`
  of the view (`roomFor`): in a channel a little longer than the window, and only a few screens
  tall, reading ahead would otherwise drop the very messages being read. The store keeps the
  window at most `WINDOW_MAX_MESSAGES` long, evicting the far end's records, except that a
  window followed live grows to `LIVE_WINDOW_MAX_MESSAGES` (twice that) before its oldest go,
  since the reader may be on the oldest of a full window when a message arrives. The Jump to
  latest pill is offered while the window lacks the newest messages, and while the view is at
  least a screen above the newest it holds (`farBack`), so a reader a few screens up in a
  channel followed live has it too; within a screen of the bottom it goes.
  On iOS and iPadOS the list scrolls itself (`OWNS_SCROLLING` in `MessageList.tsx`, with the
  arithmetic in `scrollPhysics.ts`): its box hides its overflow, so no finger, wheel, or key
  scrolls it, and the list takes those itself and sets the box's scroll position from them,
  with its own coasting, spring at the ends, and indicator. A box the browser scrolls for the
  user is the one thing that cannot be kept still there: iOS scrolls such a box in a process of
  its own and places it from where a pan began plus the finger's travel, so a position set
  from the page while a finger drags or a fling runs, by WebKit's anchoring or by script, is
  applied and then overridden by the pan's next update, and the view lands wherever the change
  put it, a page's height away. A box only scripts scroll has no such gesture, and nothing
  overrides what the list sets; scripts still scroll it, so focus, find-in-page, assistive
  technology, and Playwright bring things into view as they always did. Everywhere else the
  browser scrolls the box, on its compositor, with its own indicator, overscroll, and assistive
  gestures, and honours a position the list sets at any moment; the browser's scroll
  anchoring is off on the list on every platform, since the list keeps its own view still,
  and what it does with the position is the same either way. The Chromium history tests run
  both ways, Chromium standing in for iOS by claiming the property the list knows it by. Where the list scrolls itself, once a finger has moved `DRAG_SLOP_PX` it is dragging,
  and the rows take no pointer until it lifts, so lifting it over a picture or a button is not
  a press, as it would not be under a pan the browser made. What is in view never moves when something changes around it: the list notes a row
  and where it stands in the content (`still`), which scrolling does not change, and after
  every change, a page's arrival at commit, a picture's arrival told by the picture itself in
  the same task (`useKeepStill`), or any change of the rows' size seen by a `ResizeObserver`
  before the frame is painted, moves the position by what that row has moved. The row is the
  topmost in view, or, while a linked message is shown, that message, held by its middle so a
  jump lands on it and stays centred whatever loads around it or inside it; the reader's first
  scroll drops the link and the hold returns to the topmost row. The list scrolls itself on
  iOS for that one reason, and the browser's scrolling is the better one otherwise (it runs on
  the compositor, keeps moving while the main thread is busy, draws the platform's indicator
  and overscroll, and gives VoiceOver its three-finger scroll, which a box that hides its
  overflow does not): the conditions for giving it back there too, and the experiment that
  proves them, are in `MessageList.tsx`'s header, and nothing outside the list may assume
  either way. A page renders as a transition,
  in slices between which the finger is heard, and is fifty messages (`HISTORY_PAGE_SIZE`),
  at which committing its rows stays under a long task's 50ms where a hundred took over 120ms;
  `MessageItem` is memoized, so a change to the list renders only the rows it changes: an unmemoized row made every change to the list, a
  loading line's included, render every row through its Markdown again, synchronously, for
  half a second at a time. The browser's own scroll
  anchoring is off on the list. A picture whose size is known keeps exactly its room before it
  loads (`keptRoom` in `Attachments.tsx`), and one whose size is not keeps a square until it
  arrives. The list's own scrolls never pin or unpin it from the bottom; a script's, as
  find-in-page's, do, as the reader's do. `e2e/historyScroll.spec.ts` drags a phone back
  through 600 messages of tall pictures and paragraphs, a few pixels at a time, resting the
  finger before each lift, and fails if any step moves what is in view by other than the
  finger's distance, or if the top of what is loaded, where a page would be awaited, ever comes
  into view; a second test flicks back through the same history in quick, gathering flicks, and
  allows the awaited top to show in at most `SEAM_SHARE` of its frames; a third reads back
  through a history of short lines a little longer than the window and fails if any message in
  view leaves the page, or if anything newer is read meanwhile. Tests scroll the list as a script does, by its position, which both ways of scrolling take.
  The iOS simulator test `testReadingBackQuicklyNeverJumps` reads at a person's pace, with
  flicks and drags that begin as soon as the last ended, against a build made with
  `VITE_SCROLL_DEBUG=1`, in which the list records what it does with its position and a
  watcher samples a row in view after every painted frame, counting as a jump any frame in
  which it moved by other than what the list meant (`scrollDiagnostics.ts`, shown over the
  list by `ScrollDiagnosticsPanel` with a copy of the record); it is how the pan's override
  was found.
  A window that is not at the
  latest, whether loaded around a link or trimmed at its newer end, shows the jump control,
  which reloads the newest page.
- The UI thread is the only thread. Anything that awaits (network, storage) must not block
  rendering; keep async work in effects or event handlers and surface pending state in the UI.

## Verification before handing work back

```sh
pnpm typecheck && pnpm lint && pnpm test
```

Run `pnpm e2e` when a change touches the login flow, the layout, or anything the Playwright specs
cover. The suite runs against a stubbed server in Chromium, Firefox, and WebKit at desktop size and
on two phones (`phone-chromium`, a Pixel 7, and `phone-webkit`, an iPhone 14), which also run
`e2e/mobile.spec.ts`: the one-pane navigation, no sideways scrolling, tap-revealed message actions,
44px touch areas, and pickers that fit the screen. Specs that need a signed-in account with
communities, channels, a thread, and a DM sign in to the stubbed world in `e2e/world.ts`, which
refuses any request it does not answer with a Problem naming it. Outside CI a run uses at most
four workers, each with its own browser; the Docker suite starts its own dev server too, so run
it separately from a local run rather than alongside one. `E2E_PORT` picks the dev server's
port (5173 by default) when another one is already running there. `pnpm e2e:docker` runs the
suite, or the projects named after it (`pnpm e2e:docker --project=phone-webkit`), in Playwright's
own Docker image, which has every browser: WebKit has no supported build for most Linux
distributions (Arch among them), and this is how to run the WebKit projects there. Add
browser-specific regressions there rather than in unit tests; CI runs the Chromium desktop and
phone projects.
