# DM calls and rings

## Calls in a DM

A DM or group DM holds a call too.

- Its header's phone button starts or joins it.
- `DmCall` shows the call between the header and the messages while one is under way or the user is
  in it. It uses the same `CallStage` as a channel (see [the call screen](call-screen.md)).
- The DM's row carries a phone meanwhile.
- `CallBar` names the DM's people.
- A user card's Call button opens the DM with that person and joins its call.
- No one is offered moderation there, since the resolver gives no one Manage calls in a DM.

## Incoming rings

A call that rings the user (`RecordStore.myRings`, on any deployment they use) shows
`IncomingCall`: a modal over the whole app naming who is calling and from where.

| Control | Effect |
| --- | --- |
| Accept | Opens the DM and joins. |
| Decline, or Escape | `AspenSync.declineCall`. The modal has no X. |

While it shows:

- The ringtone (`ringtone.wav`, looped by `loopSound`; see [sounds](../notifications.md#sounds)) plays
  through the notification sound's speaker.
- When the app is not focused and the user turned system notifications on, the system notifies,
  once per ring (`ringNotifications.ts`). **Why:** a desktop may refuse a notification posted again
  in quick succession, which an effect run twice would do.
- A muted DM rings silently.
- Nothing rings in do not disturb (`IncomingCalls`, by the home's answer; see
  [Presence](../presence.md#do-not-disturb)): the servers ring no one in it, and a ring from a
  deployment that does not know it is left unshown.
- A ring ends at its `until` by the clock (`useNow`).

## Ringing out

- While the user is in a DM's call that still rings someone, the dial tone (`dial-tone.wav`, looped
  by `loopSound`) plays a quiet ringback (440 and 480 Hz, 1.2 seconds in every 4) through the voice
  chat's speaker.
- In the call, those being rung show as tiles darkened by `brightness-75`, at full opacity, with no
  visible label. A screen reader hears "Ringing".

## Call notices in the history

| Message kind | Rendered as |
| --- | --- |
| `call` | `CallNotice`, its length in words by `callLength`. |
| `missedCall` (a DM call no one else joined) | `MissedCallNotice`: "Missed call from" the caller's chip beside a red X. |
