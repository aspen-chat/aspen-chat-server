# Search

`GET /messages` searches the messages the caller may read, newest first, as far back as one request
reads. Why it reads as it does is in the [design notes](search-design-notes.md).

| Part                  | Where                                                                                                                                               |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Logic                 | `app::search`                                                                                                                                       |
| Places for deep steps | `app::deep_search`, `[search]` in `aspen.toml`                                                                                                      |
| Handler               | `api::message::search_messages`                                                                                                                     |
| Indexes               | `message_search` (words), `message_by_home_channel` (a place's history), `message_by_author` (an author's), `mention_user_recent` (a tagged user's) |
| Client                | `client/packages/app/src/features/search/`                                                                                                          |

## What is searched

- The channels the caller may view in the communities they belong to
  (`Visibility::visible_channels`).
- The DMs they are in.
- The threads of both.

Left out:

- echoes;
- poll announcements;
- messages by anyone the caller blocked.

A deployment moderator searches only what they belong to, since reading another's DM is logged where
it is opened.

## Where a message is searched

Where a message is searched is its `home_channel`:

- its own channel;
- or, for a thread reply, the thread's parent.

The trigger `message_inserted` fills it when the message is inserted. A thread's parent never
changes, so it needs no updating. A place and its threads are then one condition,
`home_channel = ANY(places)`.

A search of one thread (`filter[channel]` naming a thread) reads its replies by the channel they are
in instead (`Sought::in_thread`), through `message_channel_id_idx (channel, id)`, and skips the
probe: the thread's matches are among its parent's, which may be many more.

## Parameters

`filter[community]` or `filter[channel]` narrows where. At least one of the "what" filters must be
given.

| Parameter           | Narrows | Meaning                                             |
| ------------------- | ------- | --------------------------------------------------- |
| `filter[community]` | Where   | A community                                         |
| `filter[channel]`   | Where   | A channel, DM, or thread, with its threads          |
| `filter[text]`      | What    | Words (below)                                       |
| `filter[author]`    | What    | By author                                           |
| `filter[mentions]`  | What    | Tagged by name                                      |
| `filter[has]`       | What    | `attachment`, `image`, or `poll`                    |
| `limit`             |         | 25 by default, at most 50                           |
| `before`            |         | The `searchedTo` of the previous page               |
| `include=channels`  |         | Sideloads where each was posted, threads among them |

## Answer

The answer is the `{data, included}` envelope with two more fields:

| Field        | Meaning                                                                                                                                                                          |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `searchedTo` | How far back the search read: every match newer than it, and older than `before`, is in `data`. It names a position in history, not always a message. Null when nothing is left. |
| `more`       | Whether anything older than `searchedTo` is left to read.                                                                                                                        |

A page may hold fewer messages than `limit`, or none, and still have `more`. Only a page whose
`more` is false is the last. The next page is asked for with `before` set to `searchedTo`.

## How a page is read

No index gives the matches of a word newest first, so no statement asks for them that way. A request
reads in up to three parts, each bounded.

1. **The probe**, when there are words. Up to `PROBE_CAP` (2,000) matches are read through
   `message_search`, in no order.
   - Fewer than the cap is every match there is. The page is the newest of them that pass the other
     filters, and the search is over in this request.
   - The cap says the words are common, so matches lie near the present, and the walk finds them.
2. **The walk.** History is read backwards a window of time at a time (`Walk`). Ids are UUIDv7, so a
   stretch of time is a range of ids.
   - A window is read through the index that leads with the narrowest thing the search is of: the
     tagged user's (`mention_user_recent`), else the author's (`message_by_author`), else each
     place's (`message_by_home_channel`).
   - Every message the window holds is tested (`PASSES`), cheapest test first, the words last.
   - A window aims to hold a number of messages: 2,500 at first, four times more each step, up to
     20,000. Each window is as wide as the last suggests will hold that many. One found to hold more
     than 80,000 is read again narrower.
   - The walk reads `FREE_ROWS` (40,000) messages for any search. It stops sooner when the page
     fills, or when a window reaches the beginning of time.
3. **Going deep**, when the walk has found nothing. The walk goes on until a step finds something,
   for up to `[search] round_seconds` (20) from the request's start. Then the request answers with
   an empty page whose `more` is true, and the client asks again from `searchedTo`.

Searches without words skip the probe and walk the same way.

## Deep steps

A deep step is a core of the database for a moment, so the deployment runs only
`[search] deep_concurrency` (2) of them at once (`app::deep_search`).

- A place is held for one step, not for a search. Searches waiting on each other all advance.
- The places are advisory locks a step's transaction takes (`pg_try_advisory_xact_lock`) and gives
  back when it ends. A server that stops strands none.
- Each server lets only that many of its own searches try at once, in the order they asked. The rest
  wait holding no connection.
- Between servers a place goes to whichever asks first once it is free: a step that finds none asks
  again 25 ms later.
- A search whose client has gone away stops at its next step (`app::search::Abandoned`). Requests
  otherwise run to their end whoever is listening.
- One user goes deep in one search at a time on each server (`DeepSearches::turn`). A second search
  of theirs going deep stops the first at its next step, which answers with how far it got. So the
  places are shared among the people searching rather than their requests, and no one sending many
  searches at once takes everyone else's turns.
- A round reads for seconds, in which the caller may lose some of the places. Before its page is
  built, where they may read is worked out again, and what was found elsewhere is left out.

| Metric                                    | Counts                                                                              |
| ----------------------------------------- | ----------------------------------------------------------------------------------- |
| `aspen_deep_search_rounds_total`          | Rounds, by `outcome`: `found`, `exhausted`, `unfinished`, `abandoned`, `superseded` |
| `aspen_deep_search_step_duration_seconds` | One step, its wait for a place included                                             |

## Words

- PostgreSQL full-text search in the `simple` configuration, which lowercases and does nothing else.
  So every language is searched alike, and whole words match.
- The text takes `websearch_to_tsquery` syntax: `"a phrase"`, `or`, `-word`.
- The index `message_search` is a GIN index (with `btree_gin`) on `home_channel` and
  `to_tsvector('simple', content)`, for undeleted messages. So text is searched only within the
  places asked about, rather than through every match on the deployment.
- The search writes that expression out literally rather than binding the configuration.

**Why:** a prepared statement's generic plan then still matches the index. Binding the configuration
would stop it matching.

## Other indexes

| Index                                        | Serves                       |
| -------------------------------------------- | ---------------------------- |
| `message_by_home_channel (home_channel, id)` | A place's window of the walk |
| `message_channel_id_idx (channel, id)`       | A thread's window            |
| `message_by_author (author, id)`             | An author's window           |
| `mention_user_recent (target_user, message)` | A tagged user's window       |

Each statement is planned for the values it runs with, with settings of its own for its transaction
(`PROBE_SETTINGS`, `WALK_SETTINGS`). See the notes on them before changing a statement.

## Client

The client searches the channel or DM it is in, its community, its server, or every server the user
uses, merging the last page by page.

- A page that comes back empty with `more` is asked for again from its `searchedTo`, until one holds
  a message or nothing is left (`searchUntilFound`).
- While it waits, the dialog says how far back the search has read, and offers to stop.
- A search stopped, or failed part way, keeps where it had read to. "Show older" goes on from there.
- Across several servers, results are shown as far back as every server with more to give has read
  (`mergeResults`).
