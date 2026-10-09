# Notifications

What each user is told of is theirs to choose, for a whole community or for one channel.

| Part | Where |
| --- | --- |
| Logic | `app::notification_setting` |
| Table | `notification_setting`: the user, a community or a channel (each cascading), and a `level` |
| Default | `default_level` |
| Event | Custom `notificationSettingChanged`, on the user's own subject |
| Sideload | `include=notifications` on community reads and the DM list, as `included.notificationSettings` |
| Client rule | `RecordStore.notifies` |

## Levels

| Level | Tells of |
| --- | --- |
| `all` | Every message |
| `tags` | Only messages that tag them |
| `nothing` | Nothing |

A setting applies to a whole community, or to one text channel, DM, or channel a plugin shows. In a plugin's channel, the plugin's notices follow the setting as a message that tags them would.

## Which setting applies

1. A channel's own setting outranks its community's.
2. Without either, a DM tells of every message and a community channel of tags (`default_level`).
3. A thread follows its parent.

Every reply in a thread the user follows tells them whatever the level, though a mute of the parent still silences it (see [Threads and DMs](threads-and-dms/index.md)).

Do not disturb outranks them all: while the user is in it, nothing tells them, and no phone is woken (see [Presence](event-routing/presence.md#choosing-a-status)). The activity feed still lists what would have.

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /communities/{community}/notification-settings/@me` | Sets a community's level |
| `DELETE /communities/{community}/notification-settings/@me` | Removes it |
| `PUT /channels/{channel}/notification-settings/@me` | Sets a channel's level |
| `DELETE /channels/{channel}/notification-settings/@me` | Removes it |

## Who acts on them

- The server wakes phones by them (see [Push](push.md)).
- The apps tell of what arrives over their open streams by the same rules (`RecordStore.notifies`): someone else's unread message, not from anyone blocked, in a channel not muted, at a level that asks for it or in a thread they follow.
- The apps tell with a sound and, on the desktop and the web, the system's notifications, which the user turns on in Settings.
- None of this is Web Push: a browser tab is told only while it is open.
- The activity feed lists what this rule tells of, past and present (see [Activity feed](activity-feed.md)).
