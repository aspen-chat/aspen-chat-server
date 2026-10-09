# Access

## Deleting an account

Retiring an account (`app::user::retire`) deletes its address, its waiting mail, and its shown address with it.

## When access is given or taken away

### 1. Who can observe it, and by which routes?

| What | Who, and by which routes |
| --- | --- |
| The address and preferences | The account alone, through `GET /users/{user}/email` and `emailAccountChanged` (its own subject). |
| The shown address | Whoever reads the user: `User.publicEmail` in REST reads and sideloads, and `user` updates to everyone who shares a community with them (`UserEverywhere`). Also other deployments the user signs in to. |
| A digest | The account's own verified address, telling of channels it may view. |
| The newsletter archive | Holders of Send newsletters. |

### 2. What decides it, and where is that checked?

| What | Decided by |
| --- | --- |
| The address | It is the user themself's (`api::email::own`). |
| The shown address | `shown` and `verified_at` together, written to `user.public_email` by `set_public_email`. |
| A digest's channels | `Visibility::load` and membership in the digest's SQL, when it is made. |
| The newsletter endpoints | `DeploymentAccess::require(SendNewsletters)`. |

### 3. When it is lost, what happens to what is already open?

| Change | Effect |
| --- | --- |
| Hiding, changing, or removing the address | Clears `public_email` and announces it in the same transaction, so every open stream drops it. Clients replace the user's record. |
| Removing the address | Deletes the mail queued for it. |
| Losing view of a channel | Removes it from the next digest. A digest already made is a snapshot, and one queued in the minute before is sent as it was. |
| A ban | Skips digests and newsletters from then on. |
| Account deleted | Its mail is deleted with it. |
| A sign-out or password change | Does not stop mail. It goes to the address the account verified. |
| Losing Send newsletters | Refuses the next request. A post already sent finishes going out. |

### 4. When it is gained, how does a client already open find out?

- The `user` update with `publicEmail` reaches everyone who shares a community.
- `emailAccountChanged` tells the account's other devices to read their settings again.

### 5. Does every path that changes it announce it?

- Setting, removing, verifying, and the preferences publish inside their transactions.
- The terminal changes no addresses.
- Account deletion publishes the user's deletion.
- Unsubscribing by link announces `emailAccountChanged`.

### 6. Is it published inside the transaction that makes the change?

Yes, to the user's own subject, so a rollback is answered by `userResync`.

## Checks

`scripts/check_permissions.py` checks:

- the shown address appearing and disappearing for another member, over REST and the event stream;
- a digest leaving out a channel the reader lost view of.
