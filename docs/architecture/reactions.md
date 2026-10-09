# Reactions

A reaction is a row of `react`: the emoji, the user, the message, and when.

| Part | Where |
| --- | --- |
| Logic | `app::react` |
| Canonical emoji | `app::react::canonical_emoji` |
| Frequent emoji | `app::react::read_frequent` |
| Indexes | `react_by_message` (summaries and reactor lists), `react_by_author` (frequent emoji) |

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /messages/{message}/reactions/{emoji}/@me` | Adds the caller's reaction |
| `DELETE /messages/{message}/reactions/{emoji}/@me` | Removes it |
| `GET /messages/{message}/reactions/{emoji}` | Everyone who reacted with an emoji |
| `GET /users/@me/frequent-emoji` | The emoji the caller reacts with most |

## The emoji

- Only a single emoji is accepted.
- It is stored, compared, and named in records and events in the fully qualified form Unicode lists for it (`app::react::canonical_emoji`). So `❤` and `❤️` are one reaction.
- For a community's own emoji, see [Custom emoji](custom-emoji.md).

## Limits

| Limit | Value |
| --- | --- |
| Different emoji per message | 50 (`react::MAX_DISTINCT_PER_MESSAGE`). One more is refused with `reactTooManyDistinct` |
| Reactors per page | At most 100 |

The distinct limit is checked under a lock on the message's row, in the transaction that adds the reaction. Adding to an emoji already there is always allowed.

## Reading reactions

### Summaries

Message reads sideload reactions in brief with `include=reactions`: one `ReactionSummary` per message and emoji.

- The count.
- Whether the caller reacted.
- The first four to react, earliest first.
- Each message's emoji come in the order first used on it.

A summary stays small however popular a message is.

### Reactor lists

`GET /messages/{message}/reactions/{emoji}` lists user records, earliest first, paged with `after` (the last user of the previous page) and `limit` (at most 100).

### Rules for both

- The index `react_by_message` serves both.
- Both leave out anyone the caller blocked (see [Blocking](blocking.md)).
- A deleted message takes no reaction and lists no reactors: both answer `404`, as reading it would.

## Frequent emoji

`GET /users/@me/frequent-emoji` (`app::react::read_frequent`) gives the emoji the caller reacts with most, which the apps' quick reactions offer.

### Order

1. Those used in the past 90 days (`FREQUENT_RECENT_DAYS`), most used there first.
2. Then the rest, by how often they were used.
3. Each tie goes to the one used last.

At most 20 (`MAX_FREQUENT`); 5 unless `limit` says.

### Counting

- Counted from the caller's own rows of `react`, read newest first along `react_by_author` (`author`, `timestamp`, carrying `emoji` and `custom_emoji`).
- Stops at their latest 10,000 (`FREQUENT_WINDOW`). So the read is an index-only scan whose cost stops growing with the reactions someone has made.
- Nothing else is stored.
- A reaction taken back no longer counts.
- One on a message since deleted still does, since deleting a message keeps its reactions.

### `community`

| `community` | What counts |
| --- | --- |
| Given | Unicode emoji, and that community's own custom emoji, no other's (a custom emoji reacts only where it is defined) |
| Absent (a DM) | Only Unicode emoji |

### Access

- Only the caller reads their own. Another user's answers `403` `forbidden`.
- It tells no one anything they did not do themselves, so no change to anyone's access touches it.
- No event announces it. The apps read it again after each of the caller's own `react` events.
