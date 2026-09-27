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
  `@capacitor/app` while `@capacitor/browser` shows the page, so the native projects must
  register the `aspen` URL scheme (`CFBundleURLTypes` on iOS, an intent filter on Android) when
  they are generated; this path has not yet run on a device.
- A server that requires two-factor sign-in answers every request of an account without a
  second factor with `twoFactorEnrollmentRequired`. `AspenClient` notices it on any response
  and flags the session (`twoFactorEnrollmentRequired`), and the root layout then shows the
  enrollment screen instead of the app until a factor is added and its recovery codes have
  been dismissed (`markEnrolled`). Security changes the server says need a fresh verification
  go through `useReauth()` (`src/features/security/reauthContext.ts`), which asks the user to
  confirm it's them and tries once more.
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
  `sendMessage`. A message event carries only ids, so `useAttachment` fetches records on
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
  localized like everything else.
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
  any user control opens, and `EditProfileDialog.tsx` the signed-in user's editor.
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
  rejoins. Every `newConsumer` the server announces is consumed: audio is played through a hidden element, video is a `RemoteScreen` in `state.screens` for the app to render. `startScreenShare()` asks `VoiceMedia.getScreen()` (`getDisplayMedia` with audio) and produces the picture as `screen` and any sound as `screenAudio`, the sound as stereo Opus at 128 kbps without DTX (`SCREEN_AUDIO_CODEC`; mediasoup-client sends mono unless told, and a listener decodes whatever the producer declares); its `contentHint` option marks a picture that moves, such as a game, so the encoder gives up resolution rather than frames when bandwidth runs short; `stopScreenShare()` closes both, and the share also ends when the browser's own stop control ends the track. `state.sharingScreen` and `state.localScreen` (the preview track) describe the user's own share. In Electron, `getDisplayMedia` only works because the main process answers it in `setDisplayMediaRequestHandler` (`packages/desktop/src/main/index.ts`), with a picker as described under screen sharing on the desktop shell below. The signalling frames are the generated
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
  their volume, nothing the server or they can see), and the moderation actions. Both go
  through `AspenSync.setUserVolume` and `setUserMuted`, which apply `effectiveUserVolume` to
  whatever of theirs is playing; playback runs through a Web Audio gain node because an
  element's own volume stops at 1. `CallBar` above the user footer holds mute, deafen, share, and leave. Clicking a
  voice channel row joins it (a call the user is already in, or joining, is left alone) and
  opens `VoiceScreen`, the channel's screen in place of a
  history: the shared screens (one large, the others as thumbnails to pick), everyone in the
  call as tiles, and a Join button when the user is not in it. `ChannelHeader` is the bar both
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
  and macOS; it needs libobs development files, which on Linux and macOS `pkg-config` finds
  and on Windows `LIBOBS_INCLUDE_DIR` and `LIBOBS_LIB_DIR` name) captures one source, encodes
  it as H.264 constrained baseline, and sends it as SRTP straight to the voice server's plain
  RTP transport, answering the server's RTCP itself. With it goes the captured window's own
  sound: Windows through `wasapi_process_output_capture` and macOS through `sck_audio_capture`
  (both beta OBS features), each pointed at the same window as the picture. The audio is
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
  own libobs source `aspen_pipewire_app_audio` (`native/obs-capture/src/pipewire_audio.rs`),
  which opens a PipeWire capture stream and links one application's output ports to it,
  following that application by process id while it runs and by name across a restart, so the
  game keeps playing to its own output while the call hears it. The helper advertises no video
  kinds there, and lists the applications playing sound as `applicationAudio` in its catalogue.
- `GameCaptureDialog` (opened from `ShareControl`, the share button the call bar and the
  voice channel header both use) follows the platform. On Windows and macOS it lists the
  windows the capture source can be pointed at, with a checkbox for their sound. On Linux it
  lists the applications playing sound as a "Game audio" picker (preselecting the only one when
  just one plays), and its button opens the system's picker for the picture. Choosing the sound
  and the picture separately is a known weakness, not a design goal: one choice of "this game"
  is the better experience, and it is out of reach only because the portal tells the app
  nothing about the application behind the window the user picked. Look for ways to close
  that gap: a video track's label names the window on X11, which could identify its
  application, and a future portal may report the window's application. In development the
  dialog also offers a test pattern: the clip `ASPEN_TEST_MEDIA` names, looped through
  libobs's media source with its sound, or a colour source without one. A shell without the
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
  titles a DM with the other people's names (`useDmTitle`) and, for a group, offers
  `addDmRecipient` and `leaveDm`. `ChannelScreen` serves DMs and community channels alike.
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
- Events are routed by the server to the communities the user belongs to and to the user
  alone, so the client filters nothing itself. An event about a user reaches the client once
  per community shared with them; every copy carries the same `eventId` on its frame and
  `EventStream` drops all but the first of the last few thousand ids it has seen.
- Update events and update requests are JSON Merge Patches: an absent field is unchanged, `null`
  clears a nullable field. Apply them field by field; never replace a cached record wholesale
  with an update payload.

## UI

- Components come from `react-aria-components`. Do not reach for `react-aria` hooks or another
  component library unless React Aria genuinely lacks the primitive; if so, say why in a comment.
- Colour comes only from the semantic tokens in `src/styles.css` (`bg-surface`, `text-ink-muted`,
  `border-line`, `bg-accent`, `text-danger`, ...). Do not use Tailwind's named colours or
  `dark:` variants: each token is a `light-dark()` pair inside a palette, and a palette is a
  `[data-theme]` block of variables that `src/theme/palettes.ts` switches at runtime.
- Icon-only controls get a `Tooltip` (`src/features/layout/Tooltip.tsx`) whose text is also their
  `aria-label`, so the tooltip and the accessible name never disagree.
- Icons come from `@phosphor-icons/react` (MIT, imported by their `…Icon` names, tree-shaken).
  Prefer an icon to an emoji glyph in controls. Emoji reactions use `emoji-picker-react` with
  native glyphs, loaded lazily by `src/features/messages/Reactions.tsx`; it fetches nothing.
- Styling is Tailwind CSS 4 with `tailwindcss-react-aria-components`, so interaction states are
  the `pressed:`, `selected:`, `focus-visible:`, `invalid:` variants driven by React Aria's data
  attributes. Do not add `:hover`/`:active` CSS by hand for those states.
- User-facing strings live in `packages/app/src/i18n/messages.ts` with camelCase keys, matching
  the server's locale files. Server Problem text is already localized and is shown as-is.
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
  and dropping the segment never reloads the window. History pages in on its own as the reader
  nears either end of the loaded window (`loadOlder` / `loadNewer`), and the store keeps the
  window at most `WINDOW_MAX_MESSAGES` long, evicting the far end's records; the viewport is
  re-anchored on the topmost visible message after every change. A window that is not at the
  latest, whether loaded around a link or trimmed at its newer end, shows the jump control,
  which reloads the newest page.
- The UI thread is the only thread. Anything that awaits (network, storage) must not block
  rendering; keep async work in effects or event handlers and surface pending state in the UI.

## Verification before handing work back

```sh
pnpm typecheck && pnpm lint && pnpm test
```

Run `pnpm e2e` when a change touches the login flow or anything the Playwright specs cover. The
suite runs against a stubbed server in Chromium, Firefox, and WebKit; add browser-specific
regressions there rather than in unit tests.
