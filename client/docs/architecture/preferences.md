# Preferences

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
  named and follows `devicechange`. `NAME_COLORS`, whether names are drawn in their roles'
  colours, is device-scoped too, for a reader to whom coloured text on one screen is harder to
  read (`roles-and-permissions.md`). The palette, light or dark, and the fonts (`fonts.md`) are
  kept per install outside `PreferenceStore`, since they apply on the sign-in screen too.

## Zoom and message text size

- Zoom is the desktop app's (`src/theme/zoom.ts`), kept by the shell and applied before the
  page loads, so nothing jumps at launch: the window zooms as a browser does
  (`packages/desktop/src/main/zoom.ts`, the factor in the profile's `zoom.json`), and the main
  process takes Ctrl + and Ctrl − (⌘ on macOS), Ctrl 0, and Ctrl with the wheel before the page
  or the menu sees them and sends the page steps; the page picks the next of `ZOOM_STEPS` and
  sets it back. Nowhere else does the app zoom itself. In a browser the browser's zoom is the
  one, which a page can neither read nor set. On a phone the system's own settings are followed,
  and Settings names them in the slider's place: Android's web view follows Font size (its text
  zoom) and Display size (fewer CSS pixels across, so the layout reflows) on its own, and on iOS
  and iPadOS, whose web views draw pages at a fixed size whatever Larger Text says,
  `followSystemTextSize` (`src/theme/systemTextSize.ts`) measures a hidden probe set in the
  system's body text (`-apple-system-body`), again whenever it changes, and writes its ratio to
  the default's 17px as `--system-text-scale`; Display Zoom reaches the page as fewer points
  across. Zooming with CSS (`zoom` on the root, which iOS's `WKWebView.pageZoom` also is) is not
  used: the page keeps its full width and scrolls sideways, React Aria's popovers and the app's
  own gestures measure in unzoomed pixels, and the layout's breakpoints do not follow it.
- Every type size is redefined by `type-scale` (`styles.css`) at the element's `--text-scale`
  and `--line-scale`: at the root by the system's text size, and in `.message-text` by that
  times the message text size and line spacing below. Text grows, as the platforms' own text
  sizes grow it; spacing and icons keep their size. A phone's channel list and the like may
  shrink beside the rail (`min-w-0` on `ResizablePane`), so a large text size truncates their
  lines rather than pushing their controls off the screen.
- Contrast is kept per install with the palette and theme (`applyContrastMode` in
  `src/theme/palettes.ts`, `aspen.contrast`): System follows `prefers-contrast: more`, and
  Standard and More set it. More is `data-contrast="more"` on the root, whose block in
  `styles.css` mixes each palette's muted and faint inks, comment colour, and lines from its
  own ink and ground, so it holds in every palette; the axe audit checks the default palette at
  more contrast in both schemes.
- `MESSAGE_TEXT_SIZE` (`look.messageTextSize`, account-scoped, one of `MESSAGE_TEXT_SIZES`: 14,
  16, 18, 20, or 24 pixels) sizes messages and the message box. `useFollowMessageTextSize`
  writes it on <html> as `--message-text-scale`, and `.message-text` (`styles.css`), on the
  message list's content and the composer's field, scales each type size by it, so names,
  times, reactions, and bodies grow together while the rest of the app keeps its size. Text inside messages therefore takes Tailwind's type sizes (`text-sm`) or `em`,
  never a fixed `rem` or pixel size, which would not follow. The message box and a message
  being edited never go below 16px (`message-box-text`), since iOS zooms into a field drawn
  smaller when it is focused.
- `MESSAGE_SPACING` (`look.messageSpacing`, account-scoped, 1, 1.2, or 1.4: Normal, Wide,
  Wider) spaces the lines of messages: `useFollowMessageTextSize` writes it as
  `--message-line-scale`, which multiplies each type size's line height in `.message-text`, and
  paragraphs in a message stand apart by what a line's height adds to its text
  (`.message-body > * + *`), half the text's size at the normal spacing.
- The three are set with `StepSlider` (`src/features/layout/StepSlider.tsx`), a slider over a few
  unevenly spaced values that sets its input's value text to the value's description, since
  React Aria words a slider's value only through a number format, which would read out the
  position.
