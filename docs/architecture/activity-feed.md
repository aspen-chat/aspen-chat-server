# Activity feed

The activity feed is every message that tells a user of itself, from everywhere they read on a deployment, newest first. Apps signed in to several deployments merge each one's feed.

| Part | Where |
| --- | --- |
| Endpoint | `GET /users/@me/activity` |
| Logic | `app::activity` (`app::activity::read_activity`) |
| Client rule for new messages | `RecordStore.notifies` |
| Storage | None: the feed reads what the reader may read now, so it needs no table of its own |
| Rate limit | Its own |

## Which messages it holds

A message tells its reader by the same rule as their notifications (see [Notifications](notifications.md) and [Push](push.md)).

By the reader's level in a DM, a community, or a channel:

| Level | Messages that tell them |
| --- | --- |
| `all` (a DM's default) | Every message |
| `tags` (a community's default) | Only those that tag them: by name, a role they hold, or everyone |
| `nothing` | None |

A thread counts as its parent for the level. Besides these, every reply in a thread they follow tells them, whatever their level (see [Threads and DMs](threads-and-dms/index.md)).

A message is never in the feed when it is:

- in a channel they muted (a thread's parent included);
- by them, or by anyone they blocked;
- from before they joined (the community, or the DM a thread's parent is);
- a message that only records something: an echo, a poll's result, a call, or a command.

## How a page is read

1. The places are worked out in Rust from the user's memberships, `Visibility`, settings, mutes, and follows.
2. One query reads the page by message id, as search does (see [Search](search.md)).

## Parameters

| Parameter | Effect |
| --- | --- |
| `filter[community]` | Comma separated; keeps only those communities |
| `filter[dms]=false` | Leaves DMs out |
| `filter[unread]=true` | Keeps only what is after the reader's position where it was said (a thread's own; see [Read positions](read-positions.md)) |
| `limit` | 25 by default, at most 50 |
| `before` | Pages backwards |
| `include` | The same names as any message read. `readStates` brings the read positions of the channels and threads the messages were said in |

## When access is given or taken away

1. **Who can observe it, and by which routes?** Only the reader, through the one read. New messages reach an open feed through the event stream as any message does, the client deciding with `RecordStore.notifies`, the same rule.
2. **What decides it, and where is that checked?** `app::activity::read_activity` builds its places from `Visibility::visible_channels` and the DMs the user is in, so a channel they may not view is never searched. Threads are read through their parents.
3. **When the deciding permission is lost?** Every way of losing a channel takes its messages out of the next read. An open feed stops hearing of the channel as the event stream stops carrying it. Its rows already shown are read on demand when the store drops them, and leave when refused. A sign-out, password change, or ban ends the session.
4. **When it is gained?** The next read holds it, and events of the channel reach the open feed from then on.
5. **Does every path that changes it announce it?** It is derived from what is announced elsewhere (messages, settings, mutes, follows, memberships), with nothing of its own to announce.
6. **Is it published inside the transaction that makes the change?** It publishes nothing.
