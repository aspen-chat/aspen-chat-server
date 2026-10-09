# Verification

Every address given is mailed a six-digit code. Only a verified address receives anything but its code.

## Endpoints

| Endpoint | What it does |
| --- | --- |
| `POST /users/{user}/email/verification` | Takes the code and verifies the address. |
| `POST /users/{user}/email/verification-codes` | Sends a new code, replacing the last. |

## Codes

- A code lasts an hour.
- It is kept in Valkey under `email:verify:{user}`, beside the address it verifies, as its HMAC-SHA256 under the deployment's code key.
- The code key is `app::server_secret::CodeKey`, a secret in `server_secret` made by the first server to start. **Why:** whoever reads Valkey cannot recover a code by trying every one.
- A code for an address the account has since changed verifies nothing.
- Five wrong codes end it (`tooManyAttempts`), and a new one must be sent.
- Each code is counted before it is checked (`two_factor::count_attempt`, one script with the window's expiry). So codes sent all at once are checked no more than five times, whatever the rate limits allow.

## Security notices

A verified address receives the account's security notices. They are queued in the transaction of the change they tell of (`outbox::notify`). `outbox::notify` queues nothing where the deployment sends no mail or the address is not verified.

The notices are sent for:

- a second factor added or removed;
- the password changed or reset;
- a day's lock on wrong codes and passwords (see [Sign-in and security](../sign-in-and-security/index.md)).

## `email_verification_required`

With it on, a session of an account whose address is unverified is held, as a session owing a second factor is.

- The check is `Caller::email_unverified`, read in the same query that resolves the session token (`two_factor::EMAIL_UNVERIFIED_SQL`).
- `SessionUser` refuses the session with `emailVerificationRequired`.
- Only these reach it: the email endpoints (which take `EnrollingSessionUser`), signing out, and reauthenticating.
- Its event stream is refused at `identify`.

An open stream closes with 4428 when:

- the setting is turned on (each server's open streams follow its copy of the settings); or
- the account changes to an unverified address while it is on. Each `emailAccountChanged` of its user says whether the account now holds an unverified address (`FeedEvent::email_unverified`).

## Which accounts are bound

- An account without an address is not held. So `email_verification_required` binds an account once it gives one.
- `email_required` binds registration alone. Accounts made before it was turned on are not asked for an address.
