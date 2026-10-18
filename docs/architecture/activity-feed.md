# Activity feed

The activity feed is every message that tells a user of itself, from everywhere they read on a deployment, newest first. Other views of it read wider or narrower than that. Apps signed in to several deployments merge each one's feed.

| Part | Where |
| --- | --- |
| Endpoint | `GET /users/@me/activity` |
| Logic | `app::activity` (`app::activity::read_activity`) |
| Client rule for new messages | `RecordStore.notifies` |
| Storage | None: the feed reads what the reader may read now, so it needs no table of its own |
| Rate limit | Its own |

## Which messages it holds

A message tells its reader by the same rule as their notifications (see [Notifications](notifications.md) and [Push](push.md)).

By the reader's level in a DM, a community, a category, or a channel:

| Level | Messages that tell them |
| --- | --- |
| `all` (a DM's default) | Every message |
| `tags` (a community's default) | Only those that tag them: by name, a role they hold, or everyone |
| `nothing` | None |

A thread counts as its parent for the level. Besides these, every reply in a thread they follow tells them, whatever their level (see [Threads and DMs](threads-and-dms/index.md)).

A message is never in the feed when it is:

- in a channel they muted, or whose category they muted without keeping the channel unmuted (a thread's parent included);
- by them, or by anyone they blocked;
- from before they joined (the community, or the DM a thread's parent is);
- a message that only records something: an echo, a poll's result, a call, or a command.

## Views

`filter[view]` reads another view in place of what tells the reader (`app::activity::ActivityView`). A view holds its messages whatever the reader's levels: a level of `nothing` takes nothing out of it.

| View | Messages |
| --- | --- |
| Absent | What tells the reader, as above |
| `all` | Everything said where they read: every channel they may view, every DM, and their threads |
| `mentions` | Those of `all` that tag them: by name, a role they hold, or everyone |
| `dms` | Their DMs' and group DMs', and those DMs' threads |
| `threads` | Replies in the threads they follow |

What is never in the feed is in no view either: muted channels, their own messages, blocked authors, what was said before they joined, and messages that only record something. A view the server does not have is refused with `validation`.

The apps' Unread tab is not a view. It lists the channels and DMs whose read states say they are unread (see [Read positions](read-positions.md)), and reads `threads` with `filter[unread]=true` for the threads, whose positions no list carries.

## How a page is read

1. The places are worked out in Rust from the user's memberships, `Visibility`, settings, mutes, and follows.
2. One statement finds the page's message ids (`app::activity::page_sql`), and a second reads those messages.

Each view is some of three reads:

| Read | Of | Views |
| --- | --- | --- |
| A place whole, with its threads | `message.home_channel`, which is a thread reply's parent | What tells the reader (places at `all`), `all`, `dms` |
| A thread alone | `message.channel` | What tells the reader (followed threads), `threads` |
| What tags the reader in a place | `mention` | What tells the reader (places at `tags`), `mentions` |

PostgreSQL cannot read "the newest across these channels" in order from an index that leads with the channel. So no read asks it to. Each walks one index backwards from the newest, for one place, thread, or tagged target at a time, and stops at the page's length.

- **Places and threads.** The newest message of each is read first, one index row apiece. Only the places holding the newest of those, as many as the page is long, are then walked for a page each. A message among the newest 25 of all has fewer than 25 newer than it, so no other place can hold one.
- **Tags by name** are walked through `mention (target_user, message)`, and **tags of a role** through `mention (target_role, message)`, one walk for each role the reader holds. Both pass over tags in places not asked for.
- **Tags of everyone** are walked a place at a time through `mention (home_channel, message)`, as places are.
- A walk ends at the moment the reader joined (`aspen_uuid_floor`). A thread read for what is unread ends at their position in it.

So a page costs one index row for each place, then at most the page's length squared, however much the places hold.

`filter[unread]` over a place read whole is a filter, not a bound: a thread's position is its own, so what is read of a place is walked past until the page fills. The apps read what is unread only of threads.

The statement must be planned for the values it runs with, which Diesel's `sql_query` is. See the note on `page_sql` before changing how it is prepared.

## Parameters

| Parameter | Effect |
| --- | --- |
| `filter[view]` | Reads one of the views above |
| `filter[community]` | Comma separated; keeps only those communities |
| `filter[dms]=false` | Leaves DMs out |
| `filter[unread]=true` | Keeps only what is after the reader's position where it was said (a thread's own; see [Read positions](read-positions.md)) |
| `limit` | 25 by default, at most 50 |
| `before` | Pages backwards |
| `include` | The same names as any message read. `readStates` brings the read positions of the channels and threads the messages were said in |

## When access is given or taken away

1. **Who can observe it, and by which routes?** Only the reader, through the one read. New messages reach an open feed through the event stream as any message does, the client deciding with `RecordStore.inActivity`, each view's rule.
2. **What decides it, and where is that checked?** `app::activity::read_activity` builds its places from `Visibility::visible_channels` and the DMs the user is in, so a channel they may not view is never searched, in any view. Threads are read through their parents.
3. **When the deciding permission is lost?** Every way of losing a channel takes its messages out of the next read. An open feed stops hearing of the channel as the event stream stops carrying it. Its rows already shown are read on demand when the store drops them, and leave when refused. A sign-out, password change, or ban ends the session.
4. **When it is gained?** The next read holds it, and events of the channel reach the open feed from then on.
5. **Does every path that changes it announce it?** It is derived from what is announced elsewhere (messages, settings, mutes, follows, memberships), with nothing of its own to announce.
6. **Is it published inside the transaction that makes the change?** It publishes nothing.
