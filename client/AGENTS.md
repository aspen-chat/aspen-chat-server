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
  rejoins. Every `newConsumer` the server announces is consumed: audio is played through a hidden element, video is a `RemoteScreen` in `state.screens` for the app to render. `startScreenShare()` asks `VoiceMedia.getScreen()` (`getDisplayMedia` with audio) and produces the picture as `screen` and any sound as `screenAudio`; `stopScreenShare()` closes both, and the share also ends when the browser's own stop control ends the track. `state.sharingScreen` and `state.localScreen` (the preview track) describe the user's own share. In Electron, `getDisplayMedia` only works because the main process answers it in `setDisplayMediaRequestHandler` (`packages/desktop/src/main/index.ts`): the system picker where there is one, else the primary screen. The signalling frames are the generated
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
  sharing; `CallBar` above the user footer holds mute, deafen, share, and leave. Clicking a
  voice channel row joins it and opens `VoiceScreen`, the channel's screen in place of a
  history: the shared screens (one large, the others as thumbnails to pick), everyone in the
  call as tiles, and a Join button when the user is not in it. `ChannelHeader` is the bar both
  channel screens share. Moderation is `ParticipantMenu` on every other participant's tile and
  sidebar row: server mute or unmute (`AspenSync.muteVoiceParticipant`) and remove
  (`kickVoiceParticipant`), both `202 Accepted` calls whose effect arrives as the participant's
  own events. A `participantState` frame about the user themself overwrites `muted` and
  `deafened` in the call state, which is how a server mute shows on their own controls; a
  `kicked` frame with reason `kicked` sets `endedReason: "kicked"` for `VoiceEndedDialog`,
  `replaced` (another of their own clients took over) ends the call silently, and
  `serverStopping` rejoins.
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
  Anything a user might want to share is a route: `/communities/{id}/channels/{id}` and
  `.../messages/{id}`. Read params with `useParams`; navigate with `Link` and `Navigate` rather
  than by building URLs. A message link's id segment leaves the URL once the reader scrolls on
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
