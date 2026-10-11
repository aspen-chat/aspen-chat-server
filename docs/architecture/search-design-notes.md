# Search: design notes

Why [Search](search.md) works as it does.

## Why not one statement

- PostgreSQL's full-text index (GIN) returns matches in no order. "The newest 25 matches" as one statement has two plans, and each fails on one kind of word.
  - Read every match and sort: 11.4 s for a word in 4% of a channel of 10 million messages.
  - Walk history newest first and test each message: 64 s for a word that channel holds once.
- The planner cannot tell the two kinds of word apart. It takes every word to match 0.5% of rows.
- So the code decides. The probe tells common words from rare ones at a bounded cost, and each kind then takes the plan that suits it.

## The probe

- The cap is 2,000 because reading a match costs a row of the table. Below it, reading every match is cheaper than any walk.
- The probe applies no filter but the words and the places. A filter the index cannot decide would have it read matches without counting them, as an author's would.
- It is given 64 MB of `work_mem` so its bitmap names rows. With the default 4 MB the bitmap names pages, each then read whole: 266 ms against 51 ms.
- It is not narrowed by `before`. The index cannot decide that either, and the capped read stays bounded without it.

## The walk

- Windows are of time rather than of a count of messages. Several places cannot be read in order of id through an index that leads with the place, and a range of ids can be.
- A window's width is a guess from the last. The cap of 80,000, read in no order, bounds a guess that was wrong: nothing of such a window is kept, and it is read again an eighth as wide.
- The first step aims low (2,500 messages). A common word fills a page within it.
- The walk ends when a window reaches the beginning of time, so nothing asks first for the oldest message of each place.
- A place's window is bounded by a row comparison, and an author's by the author and a range of ids. Each form is the one the planner reads through the right index. The notes on `window_of` say what it does with the other.
- Sequential and bitmap scans are off for the walk. Left on, the planner read the whole table for one window: a capped read wants no order, and it took the window for a ninth of the table.

## Rounds

- One request reads on for at most `round_seconds`, twenty by default. Reverse proxies commonly cut a request idle for sixty.
- The server keeps reading while it finds nothing, rather than answering each step, so a search costs few requests and its rate limit stays meaningful.
- A page may be short or empty with more to come, which `more` says outright. A client that took a short page for the last would stop early.
- In a channel of 10 million messages, a word common only two years back was found in 30 s: an empty round, then a page 10 s into the next. Walking there read 4 million messages.

## Places for deep steps

- The count is the deployment's, since the database is what a step uses. A count on each server would grow with the servers.
- A place is held for a step, not a search. Five searches sharing two places all advance, each at two fifths of the speed.
- Advisory locks need no upkeep: a transaction's end gives its lock back, however it ends.
- A step asks again rather than waiting on a lock. A wait would hold a connection, and ending it at the round's deadline would mean cancelling a statement.
- There is no limit on one user's deep searches. The places bound what all of them cost together. One person with several searches open takes a larger share of the places while they last, and each ends when its client goes.

## A client that goes away

- A request's handler runs to its end whoever is listening, so that what it published is settled (`api::settle_after_request`). A search publishes nothing.
- A deep search therefore asks between steps whether its client has gone, and stops. Without that, a search abandoned at once would walk for the rest of its round.
