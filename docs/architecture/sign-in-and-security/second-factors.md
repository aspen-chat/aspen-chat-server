# Second factors

Code: `app::two_factor` (factors, recovery codes, attempt limits, `two_factor::limited`), `email::outbox::notify` (security emails).

## Factors

An account has two-factor sign-in on while it holds a second factor:

| Factor | Stored in | Details |
| --- | --- | --- |
| Confirmed authenticator app | `totp_secret` | RFC 6238, SHA-1, six digits, thirty seconds, one step of drift, each step accepted once |
| Passkey | `passkey` | See [Passkeys](passkeys.md) |

## Recovery codes

While two-factor sign-in is on, the account holds ten single-use recovery codes (`recovery_code`, SHA-256 digests only). Each is 50 random bits.

- Adding the first factor makes the codes. It also signs out every other sign-in (`login::revoke_other_sign_ins`; see [Sessions](sessions.md#what-signs-out-other-sign-ins)).
- Removing the last factor drops the codes.
- When the deployment setting `require_two_factor` is on (see [Administration](../administration/index.md)), removing the last factor is refused with `lastSecondFactor`.

## Attempt limits

Wrong authenticator codes and wrong passwords given to prove who someone is count per user in Valkey. This covers finishing a sign-in, re-verifying, and the old password when changing it.

| Window | Wrong attempts allowed | Then |
| --- | --- | --- |
| Fifteen minutes | 10 | Refused for the rest of the window |
| A day | 20 | Refused for the rest of the day, however the quarter hours fall |

A refusal is `tooManyAttempts`.

How counting works (`two_factor::limited`):

1. Each attempt is counted before it is checked, in one script that also sets each window's expiry.
2. A right attempt clears the quarter hour's count and is not counted in the day's.
3. The day's count keeps its wrong attempts.
4. So attempts sent all at once are checked no more than ten times.

When a wrong attempt reaches the day's limit, the account's verified address is told, once (`signInLocked`).

These are outside the limits:

- Recovery codes.
- Passkeys.
- A password reset by email does not use them.

**Why:** the count is per user, so someone who knows the password can lock the owner's codes for a day. The routes outside it keep a way in. See [design notes](design-notes.md#attempt-limits).

## Security emails

The account's verified address is mailed on these changes, queued in the transaction that made the change (`email::outbox::notify`):

| Change | Email |
| --- | --- |
| A second factor added | `secondFactorAdded` |
| A second factor removed | `secondFactorRemoved` |
| The password changed | `passwordChanged` |
| The password reset | See [Email](../email/index.md) |

Nothing is queued where the deployment sends no mail or the account has no verified address.

## Requiring a second factor

With `require_two_factor` on, a session of an account without a second factor can reach only:

- The security settings (see [Reverification and security settings](reverification.md#security-settings-endpoints)).
- `reauthenticate`.
- Passkey ceremonies.
- `logout`.

Everything else answers `twoFactorEnrollmentRequired`, the event stream included (close code 4403).

When the setting is turned on while such a session's event stream is open:

1. The stream closes with 4403.
2. The client is told so by the close (`EventStream`'s `onEnrollmentRequired`) or by a refused request.
3. The client shows the enrollment screen.

## What authenticators call the deployment

Authenticator apps and passkey prompts call the deployment by its display name, or "Aspen" (see [Administration](../administration/index.md)). An authenticator keeps the name it was given when it was added.
