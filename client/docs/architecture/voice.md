# Voice calls

- Voice lives in `packages/protocol/src/voice.ts`. `AspenSync.voice` is a `VoiceCall`: `join(channelId)`
  asks the API server for a join offer (`POST /channels/{channel}/voice/join`), opens the
  microphone (a refusal fails the join with `errorKind: "microphone"` before any server is
  tried or blamed; browsers refuse media on an insecure origin, which is anything but `https`
  or `localhost`), pings each candidate's `/health` at once, and tries them nearest first; a candidate that refuses the token
  or does not answer within `READY_TIMEOUT_MS` is reported to `POST /voice-servers/{server}/failures`
  and the next one is tried. Once a server answers `ready`, the call loads a mediasoup `Device`,
  asks for a send and a receive transport at once, produces the microphone, and then waits for the send
  transport's ICE and DTLS to connect (`CONNECT_TIMEOUT_MS`): the server accepting the producer
  says nothing about media, and a transport that fails or times out counts as that server's
  failure, so it is reported and the next candidate tried. A transport that fails mid-call
  rejoins. Joining another call while in one leaves the first and joins the second at once,
  and what does not depend on the call carries over so the move is quick: the microphone stays
  open (its producer is made with `stopTracks: false`, so the call, not the transport, decides
  when the track stops; it is closed when the next call only lets the user listen), and the
  loaded `Device` is kept by the router capabilities it was loaded with, which every call on a
  voice server shares, so it is loaded once rather than for every call. A rejoin keeps the
  microphone the same way. Every request the call waits on an answer to (each transport, its
  connection, each producer) waits at most `READY_TIMEOUT_MS`, and an `error` frame arriving
  first refuses it, since the voice server answers a request it will not honour with one, and
  while a join's requests are in flight that is the one refused (`refused` names its type). A refused request fails the join as
  a `VoiceRequestRefused`: `errorKind` is `refused`, with `retryAfterSeconds` when the voice
  server turned the user away for going too fast (a fatal `error` at `identify` with a wait
  counts the same), and the call bar says how long to wait. A refusal is the user's to wait
  out, so unlike a timeout it is not reported as the server's failure and no other server is
  tried. Every `newConsumer` the server announces is consumed: audio is played through a hidden element, video is a `RemoteScreen` in `state.screens` for the app to render. `startScreenShare()` asks `VoiceMedia.getScreen()` (`getDisplayMedia` with audio, at the screen's own resolution up to 4K and 60 frames a second, `SCREEN_QUALITY`) and produces the picture as `screen` (one layer allowed up to 25 Mbps at 60 fps, starting at 10 Mbps rather than climbing from a few hundred kbps: `SCREEN_ENCODING`, `SCREEN_VIDEO_CODEC`; the network's bandwidth estimate, not a guess of ours, brings it down) and any sound as `screenAudio`, the sound as stereo Opus at 128 kbps without DTX (`SCREEN_AUDIO_CODEC`; mediasoup-client sends mono unless told, and a listener decodes whatever the producer declares); its `contentHint` option marks a picture that moves, such as a game, so the encoder gives up resolution rather than frames when bandwidth runs short; `stopScreenShare()` closes both, and the share also ends when the browser's own stop control ends the track. `state.sharingScreen` and `state.localScreen` (the preview track) describe the user's own share. `startCamera()` asks `VoiceMedia.getCamera()` for the camera chosen in Settings (`video.input`, a device preference, up to 1080p at 30 frames a second, `CAMERA_QUALITY`) and produces it as `camera` (up to 4 Mbps, `CAMERA_ENCODING`), shown to the user as `state.localCamera` and moved to another camera when the choice changes; `stopCamera()` ends it. Others' cameras are `state.cameras`, apart from `state.screens`, and the call screen shows each above its owner's name in place of the avatar, the tile four columns wide where they fit and the whole row where not, the user's own mirrored; a camera goes full screen by its corner button or a double click (`ScreenTile`, without the F key, which means the shared screen). `state.canCamera` says whether the join offer allowed one (Use camera). `getCamera` tries the chosen camera and then every other, so one held by another app does not leave the user without the rest; a failure is a `CameraError` whose `failure` says why (`none` connected, access `denied`, every camera `failed`, or the voice server did not take it, `unsent`), kept as `state.cameraError` until the camera turns on, `clearCameraError()`, or the call ends, and the call bar shows it under its buttons with what to do. The Android app declares `CAMERA` (and the camera as optional hardware) so its WebView may ask for one. In Electron, `getDisplayMedia` only works because the main process answers it in `setDisplayMediaRequestHandler` (`packages/desktop/src/main/index.ts`), with a picker as described under screen sharing on the desktop shell in `screen-sharing-and-game-capture.md`. The signalling frames are the generated
  `src/generated/voiceSignal.ts` (from `voice_signal_schema.json`, which `pnpm codegen` builds by
  running the voice server with `--gen-signal-schema`). `VoiceCallState` is read with
  `useVoiceCall()`; it keeps `channelId` while `failed` so the call bar can show why. A call
  whose session ends with reason `serverLost` or `serverRemoved` (from the `voiceSessionEnded`
  event, which `AspenSync` hands to the call), or whose socket drops unannounced, rejoins after a
  random pause of at most `REJOIN_DELAY_MAX_MS` (a second); an `idle` ending sets `endedReason`,
  which `VoiceEndedDialog` shows until `acknowledgeEnd()`; an `empty` ending is the user's own
  leave. Browser media (`getUserMedia`, hidden `<audio>` elements per consumer) is in
  `browserMedia.ts` behind the `VoiceMedia` interface (`voiceMedia.ts`), loaded lazily so the protocol package
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
  own events. Muting and deafening show at once and disable the microphone's track locally
  (`#holdMicrophone`, which keeps it disabled whenever `muted` is shown), and the voice server
  never refuses them; unmuting and undeafening are only asked for (`#asked`) and show once a
  `participantState` frame about the user themself says the server did them. That frame sets
  `muted` and `deafened` in the call state, which is how a server mute shows on their own
  controls, but never shows the user less silenced than they have since asked to be, so an
  answer to an earlier request cannot reopen a microphone they just muted. A non-fatal `error`
  with `refused: "setState"` leaves the user as shown and sets `stateRefused` (with any
  `retryAfterSeconds`), which the call bar shows under its buttons until they try again,
  dismiss it (`clearStateRefusal`), or the call ends; a
  `kicked` frame with reason `kicked` sets `endedReason: "kicked"` for `VoiceEndedDialog`,
  `replaced` (another of their own clients took over) ends the call silently, and
  `serverStopping` rejoins.
