# Bans from the deployment

A ban from the deployment stops someone using it at all: its own users and visitors from elsewhere alike.

## Where it lives

| Part | Code |
| --- | --- |
| Bans | `app::user_ban` (`shut_out`, `BANNED_SQL`) |
| Stored on | `user.banned_at`, `ban_reason`, `banned_until` |
| Ending calls | `app::voice::kick_everywhere` |
| Closing streams | `FeedEvent::ends` |
| Refusing sign-in | `app::login::issue_session` |

## Endpoints

`PUT /admin/users/{user}/ban` bans. It answers `201`, or `200` when it replaces a ban that stood. `DELETE` on the same path lifts.

| Field | Meaning |
| --- | --- |
| `reason` | Optional |
| `durationSeconds` | Optional end |
| `deleteMessagesSeconds` | 3600 or 86400. Their messages anywhere on the deployment, DMs included, from the last hour or day go with the ban. Takes Remove content besides |
| `withOwner` | For a bot: ban its owner too |

## Who may ban

- Both banning and lifting take Ban users.
- They rank by deployment roles. Only someone whose highest role is below the banner's is banned, has their ban replaced, or has it lifted.
- Never the banner, and never the system account.
- So a ban of someone who outranks the caller is not theirs to lift.

## What a ban does

1. Revokes every sign-in they hold.
2. Publishes `accountBanned`, with its reason and end, to their own subject.
3. The event feed then closes each of their streams (`FeedEvent::ends`, close code 4410).
4. Takes them out of every call (`app::voice::kick_everywhere`). A join not yet reported is removed as its report is applied.
5. Sends `accountBanned` to each bot they own too, closing its streams and taking it out of its calls.

## While it stands

A ban stands from when it is made until it ends (`user_ban::BANNED_SQL`).

- Every sign-in is refused at `app::login::issue_session` with `deploymentBanned`, with the reason and end in its detail. A password sign-in learns so before any second factor is asked for.
- Any token still held resolves to no one.
- Bot tokens resolve to no one while the bot is banned itself or its owner is (`user_ban::shut_out`). The bot keeps its tokens, and they work again once the ban is lifted.
- Other deployments asking after them are told `refused` (see [Standing](../federation/standing.md)).
- They stay in their communities, so their name and what they posted still show.

## Logging and the directory

- Each ban and lift is in the moderation log (`banUser`, `liftUserBan`).
- The user directory takes `filter[banned]=true` and gives each standing ban as `ban` (see [The dashboard](dashboard.md#directories)).

## Foreign users

Holders of Ban users ban a foreign user as they ban the deployment's own. See [Standing](../federation/standing.md#bans) for how this works with federation.
