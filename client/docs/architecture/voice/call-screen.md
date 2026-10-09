# The call screen and call bar

## `CallBar`

- `CallBar` sits above the user footer.
- It shows the call's status and place, which link to the call's room.
- Under them is a row of mute, deafen, camera, share, and leave buttons that share its width
  equally.
- Under its buttons it shows camera errors, refused state changes, and how long to wait after a
  refusal (see [media](media.md#camera-failures), [muting](muting-and-moderation.md#refused-state-changes),
  and [joining](joining.md#request-timeouts-and-refusals)).
- On a narrow screen the call bar shows under the voice channel from the moment the user presses
  Join, so joining and a failure to join are visible there.

## Opening a voice channel

Clicking a voice channel row:

1. joins it (a call the user is already in, or joining, is left alone);
2. opens `VoiceScreen`, the channel's screen, in place of a history.

## `VoiceScreen` and `CallStage`

`VoiceScreen` holds `CallStage`, which a DM's call shares. It shows:

- the shared screens: one large, the others as thumbnails to pick;
- everyone in the call as tiles;
- a Join button when the user is not in it;
- a red Leave Call button at the top left while they are in it or joining. It only leaves: nobody
  can end a call for everyone.

`ChannelHeader` is the bar both channel screens share.

## Full screen

The large screen goes full screen with its corner button, a double click, or F (`ScreenTile`,
`fullScreen.ts`).

- It uses the Fullscreen API where that works, and otherwise fills the app's window.
- It is left the same ways, or with Escape.
- The mobile apps always fill the window, since Capacitor's WebView dismisses any element that asks
  for the whole screen. There the system's back gesture leaves it too.
- While full screen, the phone turns to the picture's orientation
  (`@capacitor/screen-orientation`, `useOrientationLock`) and is freed again after. A desktop refuses
  the lock and nothing changes.

Cameras go full screen by their own corner button or a double click (see
[media](media.md#showing-cameras)).
