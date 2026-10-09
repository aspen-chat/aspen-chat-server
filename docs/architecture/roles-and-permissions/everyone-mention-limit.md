# Everyone mention limit

Code: `app::everyone_limit`.

A community that gains as many members as the deployment setting `everyone_mention_limit` (200; see [Administration](../administration/index.md)) loses Mention everyone from its everyone role.

- Its owner is told so by the system account, and may give it back.
- Channel and category overrides are left as they are. **Why:** see the [design notes](design-notes.md#everyone-mention-limit).

## Once per community

It happens once per community, recorded in `community.everyone_limited_at`. The migration set it for communities already that large.

After a join commits, the server that made the join:

1. locks the community's row;
2. counts its members;
3. acts only if no one has already.

So concurrent joins that cross the limit act once. A join that finds the limit crossed acts even when the join that crossed it could not.
