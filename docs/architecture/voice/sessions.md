# Sessions and their ending

A session is the API server's record of one call. It exists from the first participant's report
until the last one leaves.

## Where it lives

| Piece                                              | Code                                     |
| -------------------------------------------------- | ---------------------------------------- |
| Applying a report                                  | `app::voice::sessions::apply_report`     |
| Choosing between a recorded and a reported session | `record_session`                         |
| Reaper                                             | the `reapVoice` job (`app::voice::reap`) |

## Applying a report

`apply_report` turns each report into rows and client events inside one transaction:

| Event              | Types                        |
| ------------------ | ---------------------------- |
| `voiceSession`     | `create`, `delete`           |
| `voiceParticipant` | `create`, `update`, `delete` |
| `voiceSpeaking`    | custom event                 |

A voice server reports a session only once it holds the room.

Community reads sideload calls with `include=voice`, as the DM list does.

## Silence and the reaper

All under `[voice]` in `aspen.toml`:

| Setting                   | Default         | Effect                                                                                                                                                                                                                 |
| ------------------------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `offer_silence_seconds`   | a minute        | A silent server is not offered to new joiners. A call bound to it is ended with `serverLost` by the next join offer for its channel (see [When a server dies](#when-a-server-dies)); the joiner gets fresh candidates. |
| `session_silence_seconds` | fifteen minutes | The reaper ends every session on a server silent this long: the calls of a dead server nobody tries to join.                                                                                                           |
| `idle_session_seconds`    | a day           | The reaper ends any call that has gone this long without ever holding two people at once.                                                                                                                              |

Nested `[voice]` keys can be set from the environment with a double underscore, for instance
`ASPEN_VOICE__IDLE_SESSION_SECONDS=30`. Tests shorten the limits this way.

- The reaper is a recurring job (`reapVoice`, every fifteen seconds; see [Jobs](../jobs/index.md)),
  which one server runs at a time.
- Each step ends at most fifty calls of each kind, each in a transaction of its own.
- Ending a call locks its session's row once its participants are gone.
- When two end one call at once (the reaper, a join offer, and a voice server's report), the second
  finds it ended. The call is recorded once.
- `session_silence_seconds` is minutes rather than seconds so a call survives a brief outage of the
  report link (NATS restarting, a voice server reconnecting), whose lost reports its next snapshot
  repairs, and minutes rather than hours since a dead server's calls stay on record, showing their
  people in them and taking the commands no one hears, for at most this long.

## When a server dies

A voice server that dies stops reporting, but the calls it held stay recorded until something ends
them. Three things do, in this order:

1. **A join offer for the channel** (`servers::join_offer`). Once the server has been silent for
   `offer_silence_seconds`, the first person to ask to join the channel's call ends the recorded
   session with `serverLost`, in a transaction of its own, before being offered fresh servers. The
   call's people are told to rejoin, as any `serverLost` does. The offer notes the lost call's
   start, starter, and whether it had company in Valkey (`voice_carried:{channel}`, kept for as long
   as a report of the new session is believed), and `record_session` takes the note when the new
   server's `sessionStarted` arrives, so the call goes on with its record as if the row had still
   been there (see [When a room is lost](#when-a-room-is-lost)). A Valkey outage loses the note, and
   the call starts afresh.
2. **A new server's `sessionStarted`**, when a joiner reaches one before any offer found the server
   silent (see [When a room is lost](#when-a-room-is-lost)).
3. **The reaper**, after `session_silence_seconds`, for the calls nobody tried to join.

The call's own participants rejoin within a second of losing their socket, which is before the
server has been silent for `offer_silence_seconds`: their first offer still names the dead server,
and their attempt fails. A client is expected to ask for another offer once `offer_silence_seconds`
have passed since the first, rather than giving up; the reference client does so once (`VoiceCall`
in `client/packages/protocol/src/voice.ts`). That second offer finds the server silent, ends the
recorded call, and names servers that can take it.

## When a room is lost

When a voice server reports a session and one is already recorded for that channel, `record_session`
decides which is the channel's call by where the recorded one is.

| Recorded session is on                                                                                                           | Result                                                                                                            |
| -------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| the reporting server itself (a room it lost, to a restart say)                                                                   | Recorded session ends with `serverLost`. Its participants rejoin, and their offers now name the reporting server. |
| a server that has stopped reporting within `offer_silence_seconds` (one the joiner could not reach, which offers no longer name) | Same as above.                                                                                                    |
| another server that still reports                                                                                                | See [Duplicate rooms](#duplicate-rooms).                                                                          |

## Duplicate rooms

A join token made while the channel had no call names up to ten servers and admits one connection to
each. Two people starting the call at once, or one person using the token twice, can open rooms on
two of them.

When the recorded call is on another server that still reports:

1. The recorded call goes on. The reported room is a second call in the channel and is not recorded.
2. Its server is sent `VoiceCommand::Close`.
3. The voice server tells everyone in it the call is closing (`kicked` with reason
   `serverStopping`).
4. Their clients rejoin and are offered the recorded call's server.
5. Every later report of the room (a snapshot, a join) finds it unrecorded and is refused or ignored
   the same way. It never displaces the call it duplicates.

## Ending reasons

Every ending publishes a `voiceSessionEnded` event with a reason just before the session's `delete`.

| Reason          | When                                                                                                                                                                            | Client rejoins?                                                                   |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `empty`         | The last participant left                                                                                                                                                       | No                                                                                |
| `serverLost`    | A snapshot left the session out, the room was lost (above), or its server stopped reporting and a join offer or the reaper ended it ([When a server dies](#when-a-server-dies)) | Yes                                                                               |
| `idle`          | `idle_session_seconds` passed without two people at once                                                                                                                        | No. The client shows a dialog saying the call was ended to free server resources. |
| `serverRemoved` | Its voice server was removed                                                                                                                                                    | Yes                                                                               |

A client whose voice server closed its place with reason `serverStopping` also rejoins.

### Rejoining

The client rejoins the channel on its own after a random delay of at most one second. **Why:** the
participants of a lost call do not all hit the API server in the same instant.
