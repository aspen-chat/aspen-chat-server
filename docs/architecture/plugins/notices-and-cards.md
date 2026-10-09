# Notices and cards

## Notices

Code: `notice::notify`, `notice::take_turn`, table `plugin_notice`.

### Who is told

`notify` tells someone of something in a channel where the plugin runs, only while:

- `channel_access` admits them; and
- their settings for the channel (a thread's parent's) would tell them of a message that tags them: not muted, level not `nothing`.

A channel of a plugin's kind may be muted and given a level as a text channel may (`app::channel_mute`, `app::notification_setting`). That is how its notices are silenced.

### How often

One plugin may notify one person at most:

| Setting | Default |
| --- | --- |
| `[plugins] notify_per_minute` | 10 |
| `[plugins] notify_per_day` | 100 |

The turns are counted in Valkey across servers (`notice::take_turn`), and let through while Valkey cannot be reached.

### What happens

1. The notice is kept in `plugin_notice` for a week (`notice::KEPT`). Reads leave out older ones, and the recurring job [`sweepExpired`](../jobs/kinds.md#sweepexpired) deletes them.
2. `pluginNotice` is published to the person's subject.
3. Their phones are woken with the push pointer `notice`, unless they are using Aspen (`app::push::notice`).

`GET /users/@me/plugin-notices/{notice}` gives a phone the plugin's name and the text in the reader's language, while they may still view the channel.

## Cards

Code: `card::Card`, `card::press`.

### Sending and updating

- `send-card` posts a message of the plugin's principal with a card. It is kept in `message.card` and carried as the message's `card`, naming its plugin.
- `update-card` changes or removes the card of a message the principal posted, announced as the message's update.

### Pressing a button

`POST /messages/{message}/card/buttons/{button}` (`card::press`):

1. Takes reading the message (`channel_access`).
2. Takes the plugin's running where the message is.
3. Calls the plugin's route `aspen/cards/{message}/{button}` as the presser (`route::answer_host`).
4. Answers with the plugin's answer.
