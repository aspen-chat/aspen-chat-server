# Standing, deletion, and bans

A foreign user stays welcome only while their home says so (phase five). This page covers the
standing check, account deletion notices, and banning foreign users.

## Where it lives

| Part                       | Code                                                                                                                              |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Standing check             | `app::federation::standing` (`MAX_USERS`, `MAX_DUE_PER_PASS`, `CONFIRM_BUDGET`, `shut_out`, `shut_out_step`, `refresh_documents`) |
| Who is due                 | `user_home_confirmed`                                                                                                             |
| Ending sessions            | `app::login::revoke_all_sessions`                                                                                                 |
| Statement size             | `jws::MAX_LENGTH`                                                                                                                 |
| Which senders are accepted | `received::Senders::AdmittedOrPinned`                                                                                             |
| Bans                       | `app::user_ban`                                                                                                                   |

## Settings

| Setting                                  | Default | Meaning                                                |
| ---------------------------------------- | ------- | ------------------------------------------------------ |
| `[federation] standing_interval_seconds` | An hour | How often foreign users are confirmed with their homes |
| `[federation] standing_grace_seconds`    | A day   | How long an unreached home is tolerated                |

## The standing pass

About every `standing_interval_seconds`, checked at least every five minutes, one API server runs a
pass: the one that claims the recurring job `confirmStanding` (see [Jobs](../jobs/index.md)).

1. It picks the homes with users due, at most `MAX_HOMES_PER_PASS` (192), the home whose user has
   been due longest first, stepping from one home to the next through `user_home_confirmed`.
2. It picks the users with a session here who are due of each, at most `MAX_DUE_PER_PASS` (5000) a
   pass, shared evenly among those homes. Of each home, those never confirmed come first, then those
   confirmed longest ago. A foreign user with no session here is not asked about: they can do
   nothing here until their home signs them in again, and asking about everyone who ever visited
   would make a pass grow with what is stored rather than with who is active.
3. It asks their homes about them, sixteen homes at a time, each chunk of a home's users within
   twenty seconds of its own, and each home within a minute in all (`HOME_BUDGET`).
4. Chunks not asked within the home's minute or the pass's four minutes (`CONFIRM_BUDGET`) are left
   due for the next pass rather than counted against their home.
5. It holds no pooled connection while it waits on a home.
6. Each request is a signed `aspen-standing-request+jwt` POSTed to the home's
   `/api/v1/federation/standing`.
7. The home answers with a signed `aspen-standing+jwt`.
8. The pass applies each answer (below). Those in good standing are confirmed together, in one
   database update.

The same pass refreshes other deployments' documents and prunes unused ones; see
[The directory](directory.md#refreshing-documents).

### Limits

- Each request names at most `MAX_USERS` (128) users, so that it and its answer fit within the 16
  KiB a statement may be (`jws::MAX_LENGTH`).
- An answer's body is read no further than that.
- A home that keeps failing is left alone for a while, the wait doubling up to an interval. Its
  users still lose their sessions at the grace.

## Answers

| Answer               | Effect                                                                                                                               |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `good`               | Confirms them (`user.home_confirmed_at`)                                                                                             |
| `gone`               | Retires them                                                                                                                         |
| `refused`            | Ends their sessions (`app::login::revoke_all_sessions`). The user left this deployment, or their home's emigration gate closed to it |
| A standing not known | Ends their sessions                                                                                                                  |
| Left out             | No news of them, as for a home not reached: they stay due, and their sessions end once unconfirmed for `standing_grace_seconds`      |

Sessions also end when:

- this deployment's own immigration gate closes to the home;
- a home goes unreached for `standing_grace_seconds`, or leaves a user out of its answers for as
  long.

Each user's sessions end in a transaction of their own (`end_stay`): the revocation is announced to
their open streams before it commits, so a failed announcement rolls it back and they stay due for
the next pass rather than being signed out with their streams never told.

A home that leaves its users out of its answers, or never answers, keeps them due, but takes no more
of a pass than its share, so every other home's users are still asked about. **Why:** see
[design notes](design-notes.md#a-user-left-out-of-an-answer-is-no-news).

## Answering as a home

A home answers only for its users who signed in at the asker (`user_foreign_deployment`).

- `gone` for those whose account was deleted.
- `refused` for any other id, whatever is true of it.

So an answer reveals nothing of accounts the asker was never given.

## Account deletion

A user who deletes their account has their home send an `accountDeleted` notice to every deployment
they used (`user_foreign_deployment`). It retires them there at once. The notices are
`deliverNotice` jobs saved in the transaction that retires the account (`app::user::retire`,
`notices::queue_deleted`), one per deployment, tried again with the jobs' backoff for about a day
when a deployment is down (see [Jobs](../jobs/kinds.md#delivernotice)).

A deployment the notice never reaches within that day learns of the deletion only through the
standing pass, which asks only about users with a session there (above): a deleted account whose
user has no session at such a deployment is never retired there, and stays a member of its
communities, shown as any other, though nobody can act as them, since their home signs in nobody for
a deleted account. **Why the pass keeps its bound:** see [design notes](design-notes.md#standing).

| Notice             | Accepted from                                                                                                                                                        |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `accountDeleted`   | Any home whose key is pinned, even one no gate admits any more (`received::Senders::AdmittedOrPinned`). It verifies against the pinned key alone and contacts no one |
| Every other notice | Only a deployment a gate admits                                                                                                                                      |

`accountDeleted` is still refused from a suspended deployment (see
[The directory](directory.md#suspension)). **Why:** see
[design notes](design-notes.md#accountdeleted-is-taken-from-any-pinned-home).

## Bans

Holders of Ban users ban a foreign user from the deployment as they ban its own (`app::user_ban`;
see [Bans from the deployment](../administration/deployment-bans.md)).

- Their sessions end.
- Signing in abroad refuses them (`federationRefused`) until the ban ends or is lifted.
- The dashboard's user directory marks users of other deployments by their domain.

A home answers `refused` for a user of its own it has banned. That ends their sessions on every
deployment they visit at its next standing check.
