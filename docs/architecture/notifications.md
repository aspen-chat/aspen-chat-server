# Notifications

What each user is told of is theirs to choose, for a whole community, a category, or one channel.

| Part        | Where                                                                                                                  |
| ----------- | ---------------------------------------------------------------------------------------------------------------------- |
| Logic       | `app::notification_setting`                                                                                            |
| Table       | `notification_setting`: the user, exactly one of a community, a category, or a channel (each cascading), and a `level` |
| Default     | `default_level`, by `Community.small`                                                                                  |
| Event       | Custom `notificationSettingChanged`, on the user's own subject                                                         |
| Sideload    | `include=notifications` on community reads and the DM list, as `included.notificationSettings`                         |
| Client rule | `RecordStore.notifies`                                                                                                 |

## Levels

| Level     | Tells of                    |
| --------- | --------------------------- |
| `all`     | Every message               |
| `tags`    | Only messages that tag them |
| `nothing` | Nothing                     |

A setting applies to a whole community, to a category's channels, or to one text channel, DM, or
channel a plugin shows. In a plugin's channel, the plugin's notices follow the setting as a message
that tags them would.

## Which setting applies

1. A channel's own setting outranks its category's, which outranks its community's (`level_of`).
2. Without any, a DM tells of every message, and so does a small community's channel; a large
   community's tells of tags (`default_level`; see
   [Large communities](roles-and-permissions/large-communities.md)). Each community's record says
   whether it is small (`Community.small`), so the apps follow the same default.
3. A thread follows its parent.
4. A category's setting may be made only for a category the user may learn of, and community reads
   leave out settings of categories they may no longer learn of.

Every reply in a thread the user follows tells them whatever the level, though a mute of the parent
still silences it (see [Threads and DMs](threads-and-dms/index.md)).

## Which messages may tell

Every kind of message may, by the rules above, but those that only record something told of already:
a thread reply's echo, a call's record, and a bot command, whose bot's answer is what tells. A
poll's result, a join, and a community growing large tell as any message does, each described by the
apps in the reader's language since it has no text of its own (`describe` in the client, and the iOS
and Android push code; see [Push](push.md)).

Do not disturb outranks them all: while the user is in it, nothing tells them, and no phone is woken
(see [Presence](event-routing/presence.md#choosing-a-status)). The activity feed still lists what
would have.

## Endpoints

| Endpoint                                                    | Does                     |
| ----------------------------------------------------------- | ------------------------ |
| `PUT /communities/{community}/notification-settings/@me`    | Sets a community's level |
| `DELETE /communities/{community}/notification-settings/@me` | Removes it               |
| `PUT /categories/{category}/notification-settings/@me`      | Sets a category's level  |
| `DELETE /categories/{category}/notification-settings/@me`   | Removes it               |
| `PUT /channels/{channel}/notification-settings/@me`         | Sets a channel's level   |
| `DELETE /channels/{channel}/notification-settings/@me`      | Removes it               |

## Who acts on them

- The server wakes phones by them (see [Push](push.md)).
- The apps tell of what arrives over their open streams by the same rules (`RecordStore.notifies`):
  someone else's unread message, not from anyone blocked, in a channel not muted (by its own setting
  or its category's), at a level that asks for it or in a thread they follow.
- The apps tell with a sound and, on the desktop and the web, the system's notifications, which the
  user turns on in Settings.
- None of this is Web Push: a browser tab is told only while it is open.
- The activity feed lists what this rule tells of, past and present (see
  [Activity feed](activity-feed.md)).
