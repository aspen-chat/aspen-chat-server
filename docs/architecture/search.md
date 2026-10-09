# Search

`GET /messages` searches the messages the caller may read, newest first.

| Part | Where |
| --- | --- |
| Logic | `app::search` |
| Handler | `api::message::search_messages` |
| Indexes | `message_search` (words), `message_by_home_channel` (searches without text), `message_by_author` (author filter) |
| Client | `client/packages/app/src/features/search/` |

## What is searched

- The channels the caller may view in the communities they belong to (`Visibility::visible_channels`).
- The DMs they are in.
- The threads of both.

Left out:

- echoes;
- poll announcements;
- messages by anyone the caller blocked.

A deployment moderator searches only what they belong to, since reading another's DM is logged where it is opened.

## Where a message is searched

Where a message is searched is its `home_channel`:

- its own channel;
- or, for a thread reply, the thread's parent.

The trigger `message_inserted` fills it when the message is inserted. A thread's parent never changes, so it needs no updating. A place and its threads are then one condition, `home_channel = ANY(places)`.

## Parameters

`filter[community]` or `filter[channel]` narrows where. At least one of the "what" filters must be given.

| Parameter | Narrows | Meaning |
| --- | --- | --- |
| `filter[community]` | Where | A community |
| `filter[channel]` | Where | A channel, DM, or thread, with its threads |
| `filter[text]` | What | Words (below) |
| `filter[author]` | What | By author |
| `filter[mentions]` | What | Tagged by name |
| `filter[has]` | What | `attachment`, `image`, or `poll` |
| `limit` | | 25 by default, at most 50 |
| `before` | | The last id of the previous page |
| `include=channels` | | Sideloads where each was posted, threads among them |

## Words

- PostgreSQL full-text search in the `simple` configuration, which lowercases and does nothing else. So every language is searched alike, and whole words match.
- The text takes `websearch_to_tsquery` syntax: `"a phrase"`, `or`, `-word`.
- The index `message_search` is a GIN index (with `btree_gin`) on `home_channel` and `to_tsvector('simple', content)`, for undeleted messages. So text is searched only within the places asked about, rather than through every match on the deployment.
- The search writes that expression out literally rather than binding the configuration.

**Why:** a prepared statement's generic plan then still matches the index. Binding the configuration would stop it matching.

## Other indexes

| Index | Serves |
| --- | --- |
| `message_by_home_channel (home_channel, id)` | Searches without text |
| `message_by_author` | The author filter |

## Client

The client searches the channel or DM it is in, its community, its server, or every server the user uses, merging the last page by page.
