# Preferences

Preferences are `PreferenceStore`, reached as `AspenSync.preferences` and read in components
with `usePreference(definition)`. The design notes are in
[preferences-design-notes.md](preferences-design-notes.md).

## Where it lives

| Part | Where |
| --- | --- |
| The store and definitions | `packages/protocol/src/preferences.ts` |
| The settings UI | `SettingsDialog` (`src/features/settings`, the gear in the user footer) |
| Audio device lists | `useAudioDevices` |
| Zoom | `src/theme/zoom.ts`, `packages/desktop/src/main/zoom.ts` |
| System text size on iOS | `followSystemTextSize` (`src/theme/systemTextSize.ts`) |
| Contrast | `applyContrastMode` (`src/theme/palettes.ts`) |
| Message text size and spacing | `useFollowMessageTextSize`, `.message-text` in `styles.css` |
| Step slider | `StepSlider` (`src/features/layout/StepSlider.tsx`) |

## Definitions and scopes

Each preference is a `PreferenceDefinition` with:

- a namespaced key;
- a scope;
- a fallback;
- a `parse` that turns whatever was stored into the value or `undefined`, so a stale entry falls
  back rather than surprising a reader.

| Scope | Stored in | Behaviour |
| --- | --- | --- |
| `device` | The install's storage (`localStorage` in a page; the sync options can supply another) | Never leaves the install |
| `account` | The server (`/users/@me/preferences`) | Loaded at bootstrap, written as a merge patch, re-fetched when a `userPreferencesChanged` event names the user |

`get` returns the same object for the same stored value, as an external-store snapshot must.

The palette (light or dark) and the fonts ([fonts.md](fonts.md)) are kept per install outside
`PreferenceStore`, since they apply on the sign-in screen too.

## Preferences

| Preference | Key | Scope | What it does |
| --- | --- | --- | --- |
| `AUDIO_INPUT` | | device | Microphone: the system default or a `NamedDevice` |
| `AUDIO_OUTPUT` | | device | Speaker: the system default or a `NamedDevice` |
| `NOTIFICATION_OUTPUT` | | device | Where notification sounds play; defaults to following the voice output |
| `NAME_COLORS` | | device | Whether names are drawn in their roles' colours ([roles-and-permissions.md](roles-and-permissions.md)) |
| `TYPING_NOTICES` | `privacy.typingNotices` | account | Whether others are told when the user is typing; on unless turned off under Privacy |
| `MESSAGE_TEXT_SIZE` | `look.messageTextSize` | account | Size of messages and the message box |
| `MESSAGE_SPACING` | `look.messageSpacing` | account | Line spacing of messages |

### Audio devices

- A `NamedDevice` keeps both id and label, since browsers salt device ids per site and the ids
  can change.
- `resolveDevice` finds the device by id, then by label.
- `notificationOutputDevice` resolves `NOTIFICATION_OUTPUT` for whatever plays notification
  sounds.
- `AspenSync` feeds the audio choices to `VoiceCall.setAudioDevices`. That swaps the microphone
  producer's track mid-call and re-routes playback with `setSinkId` where the browser has it.
- `useAudioDevices` asks for the microphone once, so devices are named, and follows
  `devicechange`.

### Name colours

`NAME_COLORS` is device-scoped for a reader to whom coloured text on one screen is harder to
read.

### Typing notices

- Turning `TYPING_NOTICES` off says at once that the user stopped typing, wherever they were.
- The user still sees others typing.

## Zoom and message text size

### Zoom

Zoom is the desktop app's (`src/theme/zoom.ts`). The shell keeps it and applies it before the
page loads, so nothing jumps at launch.

- The window zooms as a browser does (`packages/desktop/src/main/zoom.ts`). The factor is in the
  profile's `zoom.json`.
- The main process takes these keys before the page or the menu sees them, and sends the page
  steps:
  - Ctrl + and Ctrl − (⌘ on macOS);
  - Ctrl 0;
  - Ctrl with the wheel.
- The page picks the next of `ZOOM_STEPS` and sets it back.
- Nowhere else does the app zoom itself.

| Platform | What zooms |
| --- | --- |
| Desktop app | The shell's zoom above |
| Browser | The browser's own zoom, which a page can neither read nor set |
| Android | The web view follows Font size (its text zoom) and Display size (fewer CSS pixels across, so the layout reflows) on its own |
| iOS and iPadOS | `followSystemTextSize` follows Larger Text; Display Zoom reaches the page as fewer points across |

On a phone, Settings names the system's settings in the slider's place.

On iOS and iPadOS, web views draw pages at a fixed size whatever Larger Text says. So
`followSystemTextSize` (`src/theme/systemTextSize.ts`):

1. measures a hidden probe set in the system's body text (`-apple-system-body`);
2. measures it again whenever it changes;
3. writes its ratio to the default's 17px as `--system-text-scale`.

CSS zoom (`zoom` on the root, which iOS's `WKWebView.pageZoom` also is) is not used. See the
[design notes](preferences-design-notes.md#no-css-zoom).

### The type scale

`type-scale` (`styles.css`) redefines every type size at the element's `--text-scale` and
`--line-scale`:

- at the root, by the system's text size;
- in `.message-text`, by that times the message text size and line spacing below.

Text grows, as the platforms' own text sizes grow it. Spacing and icons keep their size.

A phone's channel list and the like may shrink beside the rail (`min-w-0` on `ResizablePane`).
A large text size then truncates their lines rather than pushing their controls off the screen.

### Contrast

Contrast is kept per install with the palette and theme (`applyContrastMode` in
`src/theme/palettes.ts`, `aspen.contrast`).

| Mode | Effect |
| --- | --- |
| System | Follows `prefers-contrast: more` |
| Standard | Standard contrast |
| More | Sets `data-contrast="more"` on the root |

- `data-contrast="more"` has a block in `styles.css`. It mixes each palette's muted and faint
  inks, comment colour, and lines from its own ink and ground, so it holds in every palette.
- The axe audit checks the default palette at more contrast in both schemes.

### Message text size

`MESSAGE_TEXT_SIZE` is one of `MESSAGE_TEXT_SIZES`: 14, 16, 18, 20, or 24 pixels. It sizes
messages and the message box.

- `useFollowMessageTextSize` writes it on `<html>` as `--message-text-scale`.
- `.message-text` (`styles.css`) is on the message list's content and the composer's field. It
  scales each type size by the value.
- Names, times, reactions, and bodies grow together. The rest of the app keeps its size.
- **Text inside messages takes Tailwind's type sizes (`text-sm`) or `em`, never a fixed `rem` or
  pixel size,** which would not follow.
- The message box and a message being edited never go below 16px (`message-box-text`). iOS zooms
  into a field drawn smaller when it is focused.

### Message spacing

`MESSAGE_SPACING` is 1, 1.2, or 1.4 (Normal, Wide, Wider). It spaces the lines of messages.

- `useFollowMessageTextSize` writes it as `--message-line-scale`.
- That multiplies each type size's line height in `.message-text`.
- Paragraphs in a message stand apart by what a line's height adds to its text
  (`.message-body > * + *`). At the normal spacing that is half the text's size.

### Step slider

The three are set with `StepSlider` (`src/features/layout/StepSlider.tsx`). It is a slider over
a few unevenly spaced values. It sets its input's value text to the value's description, since
React Aria words a slider's value only through a number format, which would read out the
position.
