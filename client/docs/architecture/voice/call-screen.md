# The call screen and call bar

## `CallBar`

- `CallBar` sits above the user footer.
- It shows the call's status and place, which link to the call's room.
- Under them is a row of mute, deafen, camera, share, and leave buttons that share its width
  equally.
- Under its buttons it shows camera errors, refused state changes, and how long to wait after a
  refusal (see [media](media.md#camera-failures),
  [muting](muting-and-moderation.md#refused-state-changes), and
  [joining](joining.md#request-timeouts-and-refusals)).
- On a narrow screen the call bar shows under the voice channel from the moment the user presses
  Join, so joining and a failure to join are visible there.

### Device arrows

Mute, deafen, and camera each have a small upward arrow at their end (`WithDeviceArrow`,
`src/features/voice/DeviceArrows.tsx`), which opens a popover above the bar:

| Arrow  | Popover         | Holds                                              |
| ------ | --------------- | -------------------------------------------------- |
| Mute   | `InputDevices`  | the microphone (`AUDIO_INPUT`) and its volume      |
| Deafen | `OutputDevices` | the speaker (`AUDIO_OUTPUT`) and the call's volume |
| Camera | `CameraDevices` | the camera (`VIDEO_INPUT`)                         |

- The arrow lies over the end of its button's padding, out of the layout: each control sits in a box
  that takes its equal share of the row and the control fills it, so the buttons keep the width and
  place they have without arrows. The arrow is not made larger on a touch screen.
- The mute arrow shows while a moderator has muted the user and while they only listen, since the
  microphone's choice and volume still apply. The camera's shows with the camera button.
- The popovers write the same preferences as Settings' Audio and video section, which also has the
  two volumes, so both always agree.
- Where the browser does not name devices yet, the popover shows the same notice and Allow button as
  Settings (`AccessNotice`, `src/features/settings/DeviceSelect.tsx`).

On a phone or tablet (`isMobilePlatform`: the mobile apps, or a browser on Android, iOS, or iPadOS,
whatever the window's width):

- The microphone and speaker lists show only when there are several to choose from; otherwise the
  popover holds only the volume. A browser that cannot route output offers no speaker list there.
- In the mobile apps, a phone with only its receiver and loudspeaker (`choosesSpeakerphone`) shows a
  Speaker toggle instead of a list, its icon in a filled accent circle while on. It sets
  `SPEAKERPHONE`, which `useFollowSpeakerphone` (`src/api/audioRoute.ts`) applies through the native
  `AspenAudioRoute` plugin from the call's start to its end. While anything else is connected
  (headphones, a headset, a car) the toggle is gone and the route is the system's.
- Where every camera faces front or back (`flipsBetweenSides`), however many lenses the back has,
  one button switches to the other side instead of a list: to its first listed camera, the side's
  main one (`flippedCamera`), as the camera choice. A camera's side is the browser's `facingMode`,
  or its name where the browser gives none; a name in another language places none, and the list
  shows instead.

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
- While full screen, the phone turns to the picture's orientation (`@capacitor/screen-orientation`,
  `useOrientationLock`) and is freed again after. A desktop refuses the lock and nothing changes.

Cameras go full screen by their own corner button or a double click (see
[media](media.md#showing-cameras)).
