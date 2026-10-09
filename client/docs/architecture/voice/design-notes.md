# Voice calls: design notes

Rationale behind the [voice call](index.md) pages.

## Joining

See [joining and leaving](joining.md).

- **The microphone opens before any server is tried.** A refusal of the microphone is the user's
  device, not a server's fault, so no server is blamed for it.
- **A join waits for the send transport's ICE and DTLS to connect.** The server accepting the
  producer says nothing about whether media flows, so a transport that fails or times out is the
  server's failure and the next candidate is tried.
- **A refused request is not reported as the server's failure.** A refusal (such as going too fast)
  is the user's to wait out, unlike a timeout, so no other server is tried.
- **The microphone producer uses `stopTracks: false`.** The call, not the transport, decides when the
  track stops, so the microphone stays open across a move between calls or a rejoin and the move is
  quick.
- **The mediasoup `Device` is kept by its router capabilities.** Every call on a voice server shares
  them, so loading the `Device` once rather than for every call saves time on each join.

## Media

See [media](media.md).

- **Screen shares start at 10 Mbps** rather than climbing from a few hundred kbps. The network's
  bandwidth estimate, not a guess of ours, brings it down.
- **Screen audio is declared stereo.** mediasoup-client sends mono unless told, and a listener
  decodes whatever the producer declares.
- **`contentHint` marks moving pictures.** For a game, the encoder then gives up resolution rather
  than frames when bandwidth runs short.
- **`browserMedia.ts` is loaded lazily behind `VoiceMedia`.** The protocol package stays importable
  in Node, and tests pass a fake.
- **`getCamera` tries every camera.** One held by another app does not leave the user without the
  rest.

## Playback

See [participants and volume](participants-and-volume.md).

- **Volume runs through a Web Audio gain node.** An element's own volume stops at 1, and a person's
  volume can go up to `MAX_USER_VOLUME`.
- **Each stream also plays through a muted element.** Chromium (and so the desktop app) feeds a
  remote WebRTC track into Web Audio only while the track also plays through a media element.
  Without it the graph is silent there. Firefox plays either way.

## Mute and deafen

See [muting and moderation](muting-and-moderation.md).

- **Muting is local and immediate; unmuting is asked for.** The voice server never refuses a mute
  or deafen, so they show at once. Unmuting shows only once a `participantState` frame says the
  server did it.
- **A `participantState` frame never shows the user less silenced than they have since asked.** An
  answer to an earlier request must not reopen a microphone they just muted.

## The call screen

See [the call screen](call-screen.md).

- **The Leave Call button only leaves.** Nobody can end a call for everyone.
- **The mobile apps always fill the window for full screen.** Capacitor's WebView dismisses any
  element that asks for the whole screen.
- **The narrow-screen call bar shows from the moment Join is pressed.** Joining and a failure to join
  are then visible there.

## Rings

See [DM calls and rings](dm-calls-and-rings.md).

- **The system notifies once per ring.** A desktop may refuse a notification posted again in quick
  succession, which an effect run twice would do.
- **No moderation in DMs.** The resolver gives no one Manage calls in a DM.
