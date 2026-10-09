# Password reset

A forgotten password is reset by email from the sign-in screen (`app::email::reset`), in three unauthenticated steps.

## Steps

1. **Start.** `POST /auth/password-resets` with a username (any case, as sign-in reads it).
   - It answers the reset's `id` and the account's verified address masked (`app::email::mask`).
   - It answers `passwordResetUnavailable`, with the reason, for an unknown username, an account without a verified address, a bot, or a deployment that sends no mail.
   - The reset lives in Valkey for half an hour under the digest of its id.
2. **Ask for a code.** `POST /auth/password-resets/{reset}/codes` with the whole address, compared without case.
   - When it matches, it mails an eight-digit code, kept in the reset as its HMAC under the code key, as a [verification code](verification.md) is.
   - It answers `204` whether or not it matches. The app says a code is on its way if the address was right.
   - Asking again sends a new code, replacing the last.
3. **Complete.** `POST /auth/password-resets/{reset}/completion` with the code and a new password. It:
   - replaces the password;
   - ends every sign-in of the account (`app::login::revoke_all_sessions`, which closes their event streams);
   - removes recent second factors and recovery codes (below);
   - tells the address (`passwordWasReset`, with how many factors went).

## The mask

- The first three characters before the `@` are shown, fewer when it is shorter, so one is always hidden.
- Each other character before the `@` is an asterisk.
- The domain is shown whole.

## One reset per account

Starting a reset ends the one before. `email:reset-live:{user}` names the live one, swapped in one step. So however many are started, one account holds one reset's state in Valkey.

## What completing removes

- Second factors added in the last week, and recovery codes issued in it (`two_factor::remove_recent`, by `totp_secret.confirmed_at`, `passkey.created_at`, and `recovery_code.created_at`).
- The older factors stay, so signing in still asks for one of the owner's. Codes issued before the week stay too.
- When the remaining factors are left with no codes at all, the notice says so (`recovery_codes_gone`) and asks the owner to sign in and make new ones. The reset does not make and mail codes itself.
- When no factor is left, the remaining codes go too, as removing the last factor does. The owner adds a factor again (asked to, where the deployment requires one).
- The account's bots keep their tokens. A token is reissued only by its owner with a freshly verified sign-in (see [Bots](../bots/index.md)).

[Why](design-notes.md#password-reset)

## Limits

| Limit | Value |
| --- | --- |
| Reset lifetime | Half an hour |
| Addresses per reset, right or wrong | 5, then the reset ends (`tooManyAttempts`) |
| Wrong codes per reset | 5, then the reset ends |
| Reset codes mailed per account | 5 an hour, however many resets ask |

- Once an account has been mailed five codes in the hour, asking answers `tooManyAttempts` whatever the address. This is read before the address is compared, so the answer still tells nothing of it.
- Each address and code given is counted against the reset before it is checked. So a burst sent at once is checked no more than five times, even with the rate limits suspended.
- A new password too short is refused before the code is tried.

## Rate limits

- Each step is limited per address in the `sign_in` group.
- The first step is also limited by everyone's starts together, so a crowd of addresses cannot fill Valkey with resets.
- The second and third steps are limited per reset.

## What a reset reveals

What step 1 shows confirms that a username exists, which registration's `usernameTaken` does already. The address itself stays behind its mask.
