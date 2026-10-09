# The system account

The deployment speaks to people through its own account (`app::system_account`).

## The account

- One `user` row with `system` set.
- Made the first time it is needed.
- Called what the deployment is called: its display name, or "Aspen" (see [Administration](../administration/index.md)). Renaming the deployment renames it.
- It belongs to no community, so no event of a rename reaches anyone. Those holding its record see the new name when they next read it.
- It has no password or token, so nothing signs in as it.
- Its username is `system`, outside `user_name_key` and every lookup by name, so it takes no one's name.

## Reserved names

No one may take its name, or one that looks like it, for an account of their own: at registration, a rename, or making a bot.

`app::user::validate_new_username` compares UTS #39 confusable skeletons of the names, through `unicode-security`:

- In compatibility form (NFKC).
- In two foldings of case.

So these are all refused: `SYSTEM`, `ＳＹＳＴＥＭ`, `ѕуѕtеm` with Cyrillic letters, and `systern`.

## Notices

`system_account::notify` sends a notice as an ordinary message of its one-to-one DM with the person.

- Only the system account may start that DM, and it needs no shared community.
- Push, unread counts, and search treat notices like any DM.
- A notice names `@everyone` only as code. The account holds every permission in its DM, so a tag there would count.

## What the person can do

The person reads notices and nothing more (`ChannelAccess::from_system`). Writing in the DM is answered `403`, with a detail saying they cannot be answered.

They cannot:

- DM the account.
- Block it (`systemAccountNoBlock`). Muting quiets it.
- Add it to a group DM.

Records carry `system`. Clients mark the account as the system wherever it is named, and offer no way to message, call, or block it.

## Links in notices

Notices quote names others chose: a community's, or the person whose account was deleted. So:

- Nothing in a message of the system account's is fetched for a link preview (`app::message::create_message`).
- The client shows nothing in it as a link.
- A name written as a link or an address reads as the text it is.
