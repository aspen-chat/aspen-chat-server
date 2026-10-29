# DMs across deployments and notices

DMs across deployments (phase four) are DMs on one of them, the host. The host tells each
participant's home with a signed notice.

## Where it lives

| Part                      | Code                                                          |
| ------------------------- | ------------------------------------------------------------- |
| Starting a DM             | `app::dm::open_dm`                                            |
| Notices                   | `app::federation::notices`, `POST /api/v1/federation/notices` |
| Verifying every statement | `app::federation::received::receive`                          |

## Hosting

- Whoever starts a DM picks the host among the deployments they are on.
- A deployment hosts a DM only when starting it with one of its own users in it
  (`app::dm::open_dm`). Otherwise it refuses with `federationRefused`.
- The DM may go on after its own users leave.
- Everyone in it therefore has an account on the host, where its rules apply: a shared community,
  blocks.

## The `dmJoined` notice

When a DM is started with, or gains, users of other deployments:

1. The host saves a `deliverNotice` job for each one's home (`app::federation::notices::queue_dm`)
   in the transaction that puts them in the DM, so the notice stands or falls with the DM and
   survives a restart. The runners are woken as it commits.
2. The job signs an `aspen-notice+jwt` of kind `dmJoined` and POSTs it to the home's
   `/api/v1/federation/notices`. A home that does not take it is tried again with the jobs' backoff
   for about a day (`notices::MAX_ATTEMPTS`), the notice signed afresh each time; one that refuses
   it (`4xx`) is not. See [Jobs](../jobs/kinds.md#delivernotice).
3. The home takes but ignores a `dmJoined` whose `by` it would not accept as a name and display name
   of its own.
4. Otherwise, while the user still uses the sender, the home passes it to the user's devices as the
   `foreignDmJoined` event.
5. The user signs in at the host if they are not, and reads the DM.

## Verifying statements

Statements from elsewhere, notices and assertions alike, are verified in one place:
`app::federation::received::receive`. It checks:

1. the issuer, read before anything else;
2. a gate of the right direction;
3. the pinned key;
4. the audience;
5. the lifetime;
6. single use;
7. a common protocol version.

Senders are contacted as [The directory](directory.md#verifying-a-statement-from-a-stranger)
describes. Which senders are accepted for each notice is in
[Standing](standing.md#account-deletion).

## One person across deployments

User records carry `homeId` beside `homeDomain`.

- A client knows one person across deployments by them.
- A block made on any deployment is held everywhere.
- The client believes them only in records from the viewer's own home. **Why:** any other deployment
  could name anyone as its user's home. See `client/docs/architecture/deployments.md`.
