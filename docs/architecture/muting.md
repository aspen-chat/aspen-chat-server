# Muting

A user may mute a channel for themself alone: a text channel, DM, group DM, or channel a plugin shows. In a plugin's channel, a mute silences the plugin's notices (see [Plugins](plugins/index.md)).

| Part | Where |
| --- | --- |
| Logic | `app::channel_mute` |
| Table | `channel_mute`: the user and the channel (both cascading), and `until` (`NULL` for a mute that lasts until lifted) |
| Event | Custom `channelMuteChanged`, on the user's own subject |
| Sideload | `include=mutes` on community reads and the DM list, as `included.channelMutes` (mutes in force) |

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /channels/{channel}/mutes/@me` | Mutes it. `durationSeconds` is absent or `null` for indefinitely, at most a year. `201` when it was not muted, `200` when it replaced a mute in force |
| `DELETE /channels/{channel}/mutes/@me` | Lifts it |

## Expiry

- A mute that runs out ends on each device by its own clock, with no event.
- A row whose `until` has passed is simply not read back.

## Effects

- A muted channel notifies of nothing, tags included, and wakes no phone (see [Notifications](notifications.md)).
- A thread's messages count as its parent's for muting.
- A muted channel keeps its read position, so what arrived meanwhile is unread once the mute ends.
- Clients dim it and never show it as unread in their lists.
