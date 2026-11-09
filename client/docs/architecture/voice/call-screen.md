# The call screen and call bar

## `CallBar`

- `CallBar` sits above the user footer.
- It shows the call's status and place, which link to the call's room.
- Under them are its buttons (`CallControls`), each an equal share of its row: mute, deafen,
  Speaker, camera, the camera flip, share, files, and leave, which is filled in the danger colour,
  as the call screen's Leave Call is. Those that do not apply are left out, and the next takes the
  place.
- On a phone or tablet held upright (`isMobilePlatform`, narrower than `MEDIUM_WIDTH`), too narrow
  for them all with their arrows, they stand in two rows of four columns: mute, deafen, Speaker, and
  leave, what is reached for during the call, above camera, flip, share, and files. Everywhere else,
  a sideways phone included, they share one row.
- Under its buttons it shows camera errors, refused state changes, and how long to wait after a
  refusal (see [media](media.md#camera-failures),
  [muting](muting-and-moderation.md#refused-state-changes), and
  [joining](joining.md#request-timeouts-and-refusals)).
- Where one pane shows at a time (below `md`), the call bar shows under the voice channel from the
  moment the user presses Join, so joining and a failure to join are visible there, and the call
  screen leaves out its own Leave Call (`leaveInBar`).

### Device arrows

Mute, deafen, and camera each have a small upward arrow at their end (`WithDeviceArrow`,
`src/features/voice/DeviceArrows.tsx`), which opens a popover above the bar:

| Arrow  | Popover         | Holds                                              |
| ------ | --------------- | -------------------------------------------------- |
| Mute   | `InputDevices`  | the microphone (`AUDIO_INPUT`) and its volume      |
| Deafen | `OutputDevices` | the speaker (`AUDIO_OUTPUT`) and the call's volume |
| Camera | `CameraDevices` | the camera (`VIDEO_INPUT`)                         |

- The arrow lies over the end of its button's padding, out of the layout: each control sits in a box
  that takes its share of the row and the control fills it, so the arrows take no room of their own.
  The arrow is not made larger on a touch screen.
- The mute arrow shows while a moderator has muted the user and while they only listen, since the
  microphone's choice and volume still apply. The camera's shows with the camera button, unless the
  camera flips.
- The popovers write the same preferences as Settings' Audio and video section, which also has the
  two volumes, so both always agree.
- Where the browser does not name devices yet, the popover shows the same notice and Allow button as
  Settings (`AccessNotice`, `src/features/settings/DeviceSelect.tsx`).

### On a phone or tablet

`isMobilePlatform` is the mobile apps, or a browser on Android, iOS, or iPadOS, whatever the
window's size.

- The microphone and speaker lists show only when there are several to choose from; otherwise the
  popover holds only the volume. A browser that cannot route output offers no speaker list there.
- **Speaker** (`SpeakerphoneButton`, `useSpeakerphoneButton` in `deviceControls.ts`): in the mobile
  apps, a phone with only its receiver and loudspeaker (`choosesSpeakerphone`) has a Speaker button
  on the bar, its icon in a filled accent circle while on, and the deafen popover then has no
  speaker list. It sets `SPEAKERPHONE`, which `useFollowSpeakerphone` (`src/api/audioRoute.ts`)
  applies through the native `AspenAudioRoute` plugin from the call's start to its end. While
  anything else is connected (headphones, a headset, a car) the button is gone and the route is the
  system's.
- **Camera flip** (`CameraFlipButton`, `useCameraFlip`): where every camera faces front or back
  (`flipsBetweenSides`), however many lenses the back has, a button beside the camera's switches to
  the other side, in place of the camera's arrow, whether the camera is on or off: to that side's
  first listed camera, its main one (`flippedCamera`), as the camera choice. A camera's side is the
  browser's `facingMode`, or its name where the browser gives none; a name in another language
  places none, and the camera keeps its arrow and list. Cameras are named, and so placed, only once
  the camera is allowed, so the button first appears then.
- **Files** (`FilesButton`, `FilesPanel.tsx`): the call screen does not show the call's files; a
  folder button on the bar opens them (`FilesPanel`) in a dialog, counting the offers waiting for
  the user (`useIncomingOffers`, `callFiles.ts`) so none is missed out of sight. It shows when the
  panel would.

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
