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
`favicon.ico`, `apple-touch-icon.png`, and `open-graph.png` (the link preview picture of a
deployment without an icon; `packages/app/public/`), the desktop app's icons
(`packages/desktop/build/`: sized PNGs for Linux, which also give the window its icon there, an
`.ico` for Windows, and a 1024 PNG on macOS's icon grid, from which electron-builder makes the
`.icns`), the Android launcher icons (legacy, round, and adaptive, with a monochrome layer for
themed icons), the status bar icon `ic_stat_aspen` that push notifications use, the splash
images, the favicon inlined in the server's passkey page, and the line mark
(`brand/aspen-mark-line.svg`: the same leaves and trunk traced in black strokes, no fill or bark)
that sits in the middle of every QR code. Change the mark there and run it again; never edit
its outputs by hand. It needs `rsvg-convert` and ImageMagick's `magick`.

## The server contract is generated, never hand-written

`packages/protocol/src/generated/` is produced by `pnpm codegen` from `../openapi.yaml` and
`../event_schema.json`, which the server produces with `cargo run -- --gen-openapi-schema`. Do not
edit generated files, do not declare server types by hand, and do not build a request URL as a
string. Use the typed openapi-fetch client (`client.api.GET("/api/v1/channels/{channel}", …)`)
so a server-side change surfaces as a compile error here.

When the server API changes, run `pnpm codegen:regen` and fix whatever stops compiling.

## Architecture reference

How each client feature works is written up in `docs/architecture/`, one file per feature. Read the
file for a feature before working on it, and keep it describing the code as it stands in the same
commit, as with comments.

- [`about.md`](docs/architecture/about.md): About Aspen, opened from Settings (the deployment, every version, and the license), and the Open Source Attributions page and the build-time list of every package each part ships.
- [`administration.md`](docs/architecture/administration.md): the dashboard's tabs, the permissions each needs, directories, moderation log, growth charts, registration invites, and foreign users' bans.
- [`blocking.md`](docs/architecture/blocking.md): blocks as store state, collapsed runs of blocked messages, blocked DMs, and blocks in calls.
- [`bots.md`](docs/architecture/bots.md): bot badges, developer mode and the ID wizard, the bots dialog, adding a bot, and slash commands in the message box.
- [`composer-and-drafts.md`](docs/architecture/composer-and-drafts.md): the message box's layout and keys, keeping the bottom edge, Jump to latest, drafts, and who is typing.
- [`custom-emoji.md`](docs/architecture/custom-emoji.md): a community's emoji in messages, completion, reactions, and the Emoji tab.
- [`deployments.md`](docs/architecture/deployments.md): the home and other deployments, `Deployments`, scopes, invites across deployments, protocol versions, blocks across deployments, and file transfers in calls.
- [`fonts.md`](docs/architecture/fonts.md): Inclusive Sans and Intel One Mono, the bundled Noto fallbacks and their regional Han order, emoji, and the user's own fonts.
- [`ios-app.md`](docs/architecture/ios-app.md): the iOS project and its UI tests.
- [`message-list.md`](docs/architecture/message-list.md): how the message list scrolls itself on iOS, keeps what is in view still, renders pages, moves between messages by keyboard, reads out arrivals, and the tests that hold it to that.
- [`message-rendering.md`](docs/architecture/message-rendering.md): Markdown, code highlighting, spoilers, linkifying, attachments, inline images, and video cards.
- [`notifications.md`](docs/architecture/notifications.md): notification levels, the chime, and system notifications.
- [`plugins.md`](docs/architecture/plugins.md): the plugin catalogue, annotations on messages and people, messages changed by plugins, plugins' accounts, the DM notice, the plugin settings in community settings and the dashboard, channels of a plugin's kind and their views' bridge, cards, and notices.
- [`polls.md`](docs/architecture/polls.md): poll records, votes, closing, and write-ins.
- [`preferences.md`](docs/architecture/preferences.md): `PreferenceStore`, device and account scope, audio devices, the desktop app's zoom and the phones' text sizes, the message text size and line spacing, contrast, and Settings.
- [`presence.md`](docs/architecture/presence.md): polling statuses and reporting activity.
- [`profiles-and-icons.md`](docs/architecture/profiles-and-icons.md): icon upload and cropping, profile fields, the profile card, and nicknames.
- [`push.md`](docs/architecture/push.md): waking phones: the app's subscriptions, Android's handler, and iOS's notification service extension.
- [`qr-codes.md`](docs/architecture/qr-codes.md): drawing codes with the line mark, saving them, invites' codes, the addresses links name, and the phone's scanner.
- [`rail-and-channel-list.md`](docs/architecture/rail-and-channel-list.md): drag-and-drop reordering, folders on the rail, and folded categories.
- [`reactions.md`](docs/architecture/reactions.md): reaction summaries, chips, and the reactions dialog.
- [`reports.md`](docs/architecture/reports.md): reporting messages, profiles, and nicknames, message links and their embeds, warnings, reviewing reports and their categories, and bans from the deployment.
- [`roles-and-permissions.md`](docs/architecture/roles-and-permissions.md): the client's permission resolver (kept in step with the server's through `spec/permission_vectors.json`), hidden controls, community settings, bans, access presets, member search, and pins.
- [`screen-sharing-and-game-capture.md`](docs/architecture/screen-sharing-and-game-capture.md): the desktop shell's libobs helper, Linux's application audio, the game capture dialog, and the screen picker.
- [`search.md`](docs/architecture/search.md): message search across channels, communities, and deployments.
- [`sign-in.md`](docs/architecture/sign-in.md): the signed-out screens, the server each shell signs in to, passkey ceremonies and their hand-off to the system browser, and signing in from another device by a QR code.
- [`tagging.md`](docs/architecture/tagging.md): rendering tags, completing them in the message box, and mention counts.
- [`threads-and-dms.md`](docs/architecture/threads-and-dms.md): threads, echoes, DMs and group DMs, the DM list, and the system account's DM.
- [`unread-and-muting.md`](docs/architecture/unread-and-muting.md): read states, unread marks, marking read, the New Messages line, and muting.
- [`voice.md`](docs/architecture/voice.md): `VoiceCall`: joining, media, screens and cameras, call state, per-person volume, the call screen, DM calls and rings, and moderation.

## Talking to the server

- Every request goes through `AspenClient` (`packages/protocol/src/http.ts`). It attaches the
  bearer token, refreshes the session token before it expires and on a `401`, and replays the
  request once. Do not call `fetch` directly for API traffic.
- A server that requires two-factor sign-in answers every request of an account without a
  second factor with `twoFactorEnrollmentRequired`. `AspenClient` notices it on any response
  and flags the session (`twoFactorEnrollmentRequired`), and the root layout then shows the
  enrollment screen instead of the app until a factor is added and its recovery codes have
  been dismissed (`markEnrolled`). A server that requires a verified email address does the same
  with `emailVerificationRequired` (and an event stream close of 4428): the session is flagged
  (`emailVerificationRequired`) and the root layout shows the verification screen until the
  address is verified (`markEmailVerified`). Security changes the server says need a fresh verification
  go through `useReauth()` (`src/features/security/reauthContext.ts`), which asks the user to
  confirm it's them and tries once more, and resolves to `undefined` when they decline, so an
  action that has no result of its own returns one to tell success apart. Sign-in and security
  (`SecurityPanel`) changes the password too (`PUT /users/@me/password`, which signs out every
  other session), for users of this deployment only. A form that marks a field invalid clears
  that error as soon as the field is edited, since an invalid React Aria field blocks the
  form's next submission.
- Every endpoint is rate limited and may answer `429` `rateLimited` with `Retry-After`.
  `AspenClient` retries a read once when the wait is at most `RATE_LIMIT_RETRY_MAX_MS`; a
  refused write, or a longer wait, reaches the caller as an `ApiProblemError` whose localized
  text says to slow down. Code that polls must stay well inside the server's built-in limits
  (`server/app/src/rate_limits.toml`), as presence polling does.
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
  changes (`useReorderGlide`); the thread panel and member list sliding in, and the member
  drawer sliding out and back; call tiles, the
  composer's files (with an upload progress bar, from `uploadAttachment`'s `onProgress`), the
  sync banner, and the Jump to latest pill appearing; speaking rings fading; the typing
  indicator's dots lighting in turn. Code that moves
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
  audits them in every palette and both colour schemes on Chromium (the default palette at more
  contrast too), and in the default palette
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
  More contrast (`applyContrastMode`, kept per install, following the system's
  `prefers-contrast` unless the user chose Standard or More) is `data-contrast="more"` on the
  root, which draws every palette's muted and faint inks and lines nearer its ink; a token
  added to a palette that text or edges use needs its more-contrast value there too.
- Colour is never the only way something is told. Statuses are shapes (`PresenceMark`: a dot,
  a crescent, a ring), a link inside text is underlined, and a choice among tinted options is
  ticked. Under forced colours (Windows' contrast themes), which flatten backgrounds and drop
  box shadows and so Tailwind's rings, `styles.css` outlines focus and whatever is selected,
  current, or pressed; a mark drawn only as a fill (a track, a progress bar, a dot) takes
  `forced-fill`, and one drawn only as a ring (someone speaking, a chosen picture) takes
  `forced-outline`.
- Type comes only from `font-sans` and `font-mono` (`--font-sans` and `--font-mono` in
  `src/styles.css`), never a family named in a component: the user may choose their own font
  for either (`docs/architecture/fonts.md`), and the stacks hold the bundled fallbacks for every
  script and for emoji.
  Text is sized with Tailwind's type scale (`text-sm`) or `em`, never a fixed `rem` or pixel
  size (but for marks drawn to fit a box of their own: a badge's count, a picture's initials),
  so the system's text size (Larger Text on iOS) and, inside messages and the message box
  (`.message-text`), the reader's message text size reach it (`preferences.md`).
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
  Below Tailwind's `lg` breakpoint, where there is no room for the member list beside a channel,
  it is a drawer over the channel instead (`Drawer`, `src/features/layout/Drawer.tsx`, a modal
  at the inline end, or at the bottom for a sheet): the channel header's members
  button opens it, and a finger swiping across the channel toward the inline start draws it out
  and back toward the end puts it away (`useSwipe`, `src/features/layout/useSwipe.ts`). It
  follows the finger and, as the finger lifts, goes the way it was thrown or, if it came to
  rest, whichever way it is more than half; where motion is reduced it fades in and out in
  place. A finger moving mostly up or down, or across something that scrolls sideways, is left
  to scroll, and a swipe's moves stop at the swipe, so the message list, which follows a
  finger itself on iOS, does not move with one. The Android back button puts the drawer away.
- Copy to the clipboard with `copyText` (`src/features/layout/clipboard.ts`), never
  `navigator.clipboard` directly: the Clipboard API is missing on a page served over plain HTTP,
  such as a dev server a phone reaches by address, and `copyText` falls back to a copy that
  works there and in iOS Safari.
- Save a file with `saveFile` (`src/features/layout/saveFile.ts`), never a download link of
  your own: the mobile apps' web views cannot follow one, and `saveFile` hands the file to the
  system's picker there.
- Nothing may depend on hover, which a touch screen does not have. A control revealed on hover is
  also revealed by focus, or offered another way on a touch screen: a message's actions
  (`MessageActions`, icons at `ACTION_ICON`) show on hover or focus where there is a pointer, in
  a bar positioned out of the layout and rising over the message before (as far as the top of
  the list or the row's container leaves room), so the header stays one line of text tall and
  the body sits right under it, and on a touch-only device (`TOUCH_ONLY`) a long press on the
  message (`useLongPress` from `react-aria`, the one hook taken from it, since the components
  package has no long press) opens them in a sheet sliding up from the bottom over a darkened
  screen (`MessageActionSheet`, a `Drawer` at the `bottom` edge, which a finger pulls down to
  put away), with a tap felt in the hand in the apps (`haptics.ts`, `@capacitor/haptics`):
  the actions are a list of icons with their names beside them (`MessageActions` given
  `open`, each through `IconAction`), and above it the quick reactions, the reader's five most
  used emoji (`useFrequentEmoji`, topped up by `quickReactions`' defaults) and an unnamed Add a
  reaction button, which is not in the list again, then when the message was sent. The press owns the message there, so the
  browser's text selection is off on it and Copy text is among the actions. Every action
  closes the sheet: the ones that open something (the reaction picker, who reacted, delete)
  open it in the sheet's place, as `MessageItem`'s own, the picker as a popover anchored to
  the point pressed (below it, or above it in the lower half of the list and always on the
  newest message, which sits on the message box), since a popover or dialog inside the sheet
  would go with it (`MessageSheet`, `ReactionPickerPopover`, `ReactionsDialog`,
  `DeleteMessageModal`). Otherwise a control is shown outright with
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
  `planesModalClass`, `widePlanesModalClass`, or `columnPlanesModalClass` for the modal,
  `planeClass` for a section (`planeSurfaceClass` for one that lays out its own contents, such
  as a list), and `PlaneColumns` to let planes stand in two or three columns where the modal is
  wide enough (`columnPlanesModalClass` is wide enough for three). Settings,
  Sign-in and security, community settings, the bots dialog, the user card, and the
  Administration Dashboard's sections are drawn this way.
  Every modal is titled with `DialogHeading` (`src/features/layout/DialogHeading.tsx`), which
  puts a Phosphor X in its top right that closes it through the `Dialog`'s `close` slot, and
  on a later step of a stepped dialog (`OptionButton`s first, as Add new) an arrow before the
  title back to the choice (`onBack`). It lays out every control its row holds, so it is never
  wrapped in a row of someone else's, where the title would stop taking the width that puts the
  X in the corner; a control a title needs beside it is a prop of `DialogHeading`. A modal
  has no other button that only closes it (no Cancel, Close, Done, or OK); whatever closing
  must do (answering the screen picker, abandoning a crop) goes in the overlay's
  `onOpenChange`, which the X, Escape, and a click outside all reach. The exceptions are a
  modal that must not be left before the reader acts, such as the recovery codes, and an
  incoming call (`DialogHeading closeButton={false}`), whose Accept and Decline are the ways
  out; a click beside it does nothing, and Escape declines. A sheet of choices at the bottom of the
  screen (`Drawer` at the `bottom` edge, as a message's actions on a touch screen) is a menu
  more than a dialog: it is named for assistive technology alone, shows a handle rather than a
  title, and is put away by a tap above it, a pull down, Escape, or the Android back button.
- User-facing strings live in `packages/app/src/i18n/messages.ts` (keys outside any namespace) and `packages/app/src/i18n/en/<namespace>.ts` (one file per namespace, which `messages.ts` assembles into `en`), with camelCase keys, matching
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
- Three builds of the same code: `pnpm build` (web, served from a site root, real URL paths),
  `pnpm build:shell` (`--base ./`, used by the mobile package, which loads the bundle from an
  app-local origin and routes after a `#`), and `pnpm build:desktop` (the same for the desktop
  package's `file://` page). Both write their Content Security Policy into the page
  (`shellPolicy` in `vite.config.ts`), since neither page comes with headers; the web build's
  comes from the server. Never write an absolute `/assets/…` URL by hand; let Vite resolve assets so every
  build works.
- Routing is TanStack Router (`src/router.tsx`), code-based, one route tree for every shell.
  Anything a user might want to share is a route: `/communities/{id}/channels/{id}`,
  `.../messages/{id}`, and `.../threads/{id}`, and the same under `/dms/{id}` for DMs. Read
  params with `useParams`; navigate with `Link` and `Navigate` rather than by building URLs.
  The message components live under both trees, so they take the channel's home (a community
  id, or `null` for a DM) and build their links with `src/features/messages/links.ts`. A message link's id segment leaves the URL once the reader scrolls on
  their own (wheel, touch, scrollbar, or a navigation key, tracked by `ListScroller`); scroll
  events the browser fires for layout changes, image loads, or scripted scrolling do not count,
  and dropping the segment never reloads the window. History pages in on its own, well ahead
  of the reader (`useHistoryPaging`, through `AspenSync`'s `loadOlder` / `loadNewer`,
  `HISTORY_PAGE_SIZE` messages at a time): within
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
  channel followed live has it too; within a screen of the bottom it goes. A window that is not at the
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
communities, channels, a thread, and a DM sign in to the stubbed world in `e2e/world.ts` (its records, sub-worlds, and routes are under `e2e/world/`), which
refuses any request it does not answer with a Problem naming it. Outside CI a run uses at most
four workers, each with its own browser; the Docker suite starts its own dev server too, so run
it separately from a local run rather than alongside one. `E2E_PORT` picks the dev server's
port (5173 by default) when another one is already running there. `pnpm e2e:docker` runs the
suite, or the projects named after it (`pnpm e2e:docker --project=phone-webkit`), in Playwright's
own Docker image, which has every browser: WebKit has no supported build for most Linux
distributions (Arch among them), and this is how to run the WebKit projects there. Add
browser-specific regressions there rather than in unit tests; CI runs the Chromium desktop and
phone projects.
