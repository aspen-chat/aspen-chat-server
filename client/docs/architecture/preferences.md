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
