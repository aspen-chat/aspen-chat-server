# Large communities

A community becomes large when it gains as many members as the deployment setting
`large_community_members` (200; see [Administration](../administration/index.md)). From then on:

- its everyone role no longer holds Mention everyone;
- its channels tell members who chose nothing for them of tags, not every message (see
  [Notifications](../notifications.md#which-setting-applies));
- its default channel says so in a `communityGrew` message that tags everyone;
- its owner is told by the system account how to give Mention everyone back, when the role held it.

Channel and category overrides are left as they are. **Why:** see the
[design notes](design-notes.md#large-communities).

| Part         | Where                                                                                                                                             |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Logic        | `app::large_community`                                                                                                                            |
| Column       | `community.large_at`, `NULL` while it is not large                                                                                                |
| Wire         | `Community.small`: whether it is small, by the threshold of the moment (absent from a deployment that predates it, whose channels tell of tags)   |
| Message kind | `communityGrew` (`community_grew`), by the system account, tagging everyone, with no content; `growth` holds `members` and `mentionEveryoneTaken` |
| Client       | `CommunityGrewNotice` (`client/packages/app/src/features/messages/`), `communityDefaultLevel` (`client/packages/protocol/src/notifyRules.ts`)     |

## Small and large

A community is small while `large_at` is `NULL` and the threshold is above 0
(`large_community::small`). With the threshold at 0 no community crosses it and none is small, so
every community's channels tell of tags; no message is posted and no permission taken.

A small community's channels default to every message. That makes each of its messages a candidate
to wake every member's phone (`app::push`), work bounded by the threshold, since a community at it
is no longer small.

## Once per community

It happens once per community, recorded in `community.large_at`. The `everyone_mention_limit`
migration set it for communities that already had 200 members, and `bench seed` sets it for the
communities it seeds with at least the threshold.

After a join commits, the server that made the join:

1. locks the community's row;
2. counts its members;
3. acts only if no one has already.

So concurrent joins that cross the threshold act once. A join that finds the threshold crossed acts
even when the join that crossed it could not. A community that loses members afterwards stays large,
and a threshold raised later does not make it small again.

Acting, inside one transaction:

1. sets `large_at`, and announces the community's `update` with `small: false`;
2. takes Mention everyone from the everyone role, announced as the role's `update`
   (`role::take_from_everyone`);
3. posts the `communityGrew` message in the default channel, when the community has one, recording
   `mention` rows as any message's tags are.

The owner's notice is sent after it commits.

## Who may see what

1. **Observing.** `Community.small` is part of the community's record, read and received by every
   member (`EventScope::Community`). The `communityGrew` message is a message of the default
   channel: only those who may view it receive it, in REST reads, on the event stream
   (`EventScope::Channel`), through push, and in the activity feed. It is left out of search, as a
   join message is.
2. **Deciding.** Reading the message takes viewing its channel (`channel_access`). Its tag of
   everyone counts for everyone who can see the channel, as any `@everyone` does; the system
   account's tag is not checked against a permission, since the server writes it.
3. **Losing access.** Losing the view of the default channel is losing a channel: the announcement
   and anything later stop reaching the member. Being large gives no one anything, so nothing
   already open needs rechecking.
4. **Gaining access.** A member who can see the default channel later finds the announcement there.
   A client already open learns that the community is large from its `update`, and changes the level
   its channels show and notify by at once.
5. **Every path announces.** Only a join makes a community large, and it announces the change.
   Changing the threshold to or from 0 changes `small` for communities without announcing it to each
   one (that would be work in the number of communities): open clients notify by the old default
   until they next read the community, while push follows the threshold at once.
6. **Inside the transaction.** The `update`, the role's change, and the message are all published
   inside the transaction that sets `large_at`.
