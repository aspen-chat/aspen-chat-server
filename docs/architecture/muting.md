# Muting

A user may mute, for themself alone, a text channel, DM, group DM, or channel a plugin shows, or a whole category. In a plugin's channel, a mute silences the plugin's notices (see [Plugins](plugins/index.md)).

| Part | Where |
| --- | --- |
| Logic | `app::channel_mute`, `app::category_mute` |
| Tables | `channel_mute`: the user and the channel (both cascading), `until` (`NULL` for a mute that lasts until lifted), and `muted` (`false` for a channel kept unmuted); `category_mute`: the user and the category (both cascading), and `until` |
| Events | Custom `channelMuteChanged` and `categoryMuteChanged`, on the user's own subject |
| Sideload | `include=mutes` on community reads, as `included.channelMutes` and `included.categoryMutes`, and on the DM list, as `included.channelMutes` (settings in force) |
| Whether a channel is muted, in SQL | `app::channel_mute::muted_sql!` |

## A channel's mute and its category's

A channel is muted for a user when:

1. its own row is in force and says `muted`; or
2. it has no row of its own in force, and its category is muted.

So each channel is in one of three states of its own: no setting (it follows its category), muted (with its own end), or kept unmuted (`muted` false, which lasts until the user changes it). A channel moved into a muted category is muted from then on, and one moved out leaves the mute behind, since the category is read from the channel when it is asked about. A thread's messages count as its parent's.

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /channels/{channel}/mutes/@me` | Mutes it. `durationSeconds` is absent or `null` for indefinitely, at most a year. With `muted: false` it keeps the channel unmuted instead, which takes no duration. `201` when it had no setting in force, `200` when it replaced one |
| `DELETE /channels/{channel}/mutes/@me` | Removes its own setting, so it follows its category |
| `PUT /categories/{category}/mutes/@me` | Mutes the category, as a channel is muted. Only a category the user may learn of (`app::category::viewed_category`) |
| `DELETE /categories/{category}/mutes/@me` | Lifts it |

`channelMuteChanged` carries `muted`: `true` for a mute, `false` for kept unmuted, and `null` once the channel has no setting of its own.

## Expiry

- A mute that runs out ends on each device by its own clock, with no event.
- A row whose `until` has passed is simply not read back. Rows are never swept: each is one per user and channel or category, replaced by the user's next setting there and removed with the channel, the category, or the user.

## Effects

- A muted channel notifies of nothing, tags included, and wakes no phone (see [Notifications](notifications.md)).
- A muted channel keeps its read position, so what arrived meanwhile is unread once the mute ends.
- Clients dim it and never show it as unread in their lists.
- Community reads leave out mutes of categories the user may no longer learn of.
