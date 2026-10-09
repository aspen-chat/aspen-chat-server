# Tagging

A message's text can tag members, roles, and everyone who can see the channel. Which tags notify is [Notifications](notifications.md)' to decide.

| Part | Where |
| --- | --- |
| Parsing | `app::mention::parse`, which reads the message through `app::markdown` (the Markdown dialect the client renders, as the link preview scan also does) |
| Stored set | `Message.mentions`, stored as `message.mentions` JSONB through `FromSql`/`ToSql` on `app::mention::Mentions` |
| Rows | One per tag in the `mention` table |
| Unread count | `ReadState.mentions` |

## Syntax and permissions

| Tag | Syntax | Channel permission | Held by |
| --- | --- | --- | --- |
| A member | `<@user-id>` | Mention members | The member template |
| A role | `<@&role-id>` | Mention roles | The Moderator and Admin templates |
| Everyone who can see the channel | `@everyone` | Mention everyone | The Moderator and Admin templates |

A tag counts wherever the message renders as text, never in code of any kind.

## What counts

- A tag its author may not make is left as plain text and counts for no one.
- So is a tag naming someone outside the community (or DM), or a role not of it.
- At most `MAX_TAGS` (50) people and roles count per message.
- What counts is `Message.mentions` (`{users, roles, everyone}`) and a row per tag in `mention`.
- Both are written in the transaction that posts or edits the message.
- An edit of the text tags afresh, with its author's permissions of the moment, and announces the new set in the message's `update`.

## Reading

- Message reads sideload the people tagged with `include=mentions`, as `included.users`, for a reader with no other source of names (a phone's notification code).

## Unread tags

Each `ReadState` carries `mentions`: how many unread messages tag its reader (see [Read positions](read-positions.md)).

- A message counts if it tags them by name, through a role they hold now, or as everyone.
- Their own messages, and those of anyone they blocked, are left out.
- It is counted through `mention`'s indexes on who is tagged: one branch each for the reader, everyone, and each role they hold.
- Only messages from the read position on are counted, and the count stops at `MAX_COUNTED_MENTIONS` (100).
- Tags count in muted channels too.
