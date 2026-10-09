# Members

## The member sample

Every member reads the member sample: 100 members (`MEMBERS_PER_COMMUNITY`). It is all a large community ever sends its members.

`app::community::read_community_members` chooses it by priority:

1. Members with a connection, online or away (`app::user_status::connected_members`), who hold a role shown apart, by the rank of their highest such role, even when they fill the sample.
2. The rest of the connected.
3. Everyone offline, whatever their roles.

Each tier is ordered by when they last came online (`user.last_seen_at`, written as the presence key is set afresh, and beside it on each of their memberships, `community_user.last_seen_at`). The caller's own membership is always among the sample.

### Its cost

The sample's cost grows with who is connected and with the sample, not with the community:

- the connected are ranked from the (community, member) pairs presence named;
- the rest are each community's first few, through `community_user_recent`.

### Reading it again

Presence is not announced, so the sample is as of its reading. A client (`AspenSync`) reads it again, at a moment spread over ten seconds, when it hears that:

- a role came to be shown apart, or stopped being;
- a role shown apart moved or was deleted;
- a role shown apart was given or taken.

## Member search

`GET /communities/{community}/members` with `filter[name]`, `after`, or `limit` searches every member by name instead (`app::community::search_community_members`).

- A name matches on any part of the member's username, display name, or nickname there. For one or two characters, it matches the start of one.
- A page holds at most 50, the members after the member `after`.
- Members are ordered by the name the community shows, through the index `community_user_by_shown_name`.
- Matching uses the trigram index `community_user_search` on each membership's `shown_name` and `search_name`.
- The triggers `community_user_names` and `user_names_changed` keep `shown_name` and `search_name` current through every change to a nickname or a profile's names.
- Each member's roles are in `included.userCommunities`.

In a community larger than the sample, only these may search:

- the owner;
- deployment moderators;
- holders of Assign roles, Remove members, Manage channels, or Manage categories.

So no ordinary member can list a large community whole.

## One member

`GET /communities/{community}/members/{user}` reads one member's roles (`read_community_member`). Any member may, since whoever reads someone's messages may see what roles they hold.
