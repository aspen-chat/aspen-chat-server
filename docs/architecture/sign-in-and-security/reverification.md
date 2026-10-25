# Reverification and security settings

## Recent verification

A sign-in records when it last proved who its user is, in `refresh_token.verified_at`: the moment it
was made for a password, a passkey, or a second factor, and for a sign-in through an
[identity provider](identity-providers.md#signing-in) the provider's `auth_time` (never later than
now), or never when it sent none.

These changes need that to be within `[auth] reverify_seconds`, or they answer
`reauthenticationRequired`:

- Adding or removing a factor.
- New recovery codes.
- Deleting the account.
- Changing the password of an account with two-factor sign-in on.

Other features ask for it too: giving a sign-in by a
[device link](device-links.md#who-may-give-a-sign-in), and signing out everywhere else
([Sessions](sessions.md#what-signs-out-other-sign-ins)).

## Refreshing it

`POST /auth/reauthenticate` refreshes `verified_at`:

- With the password.
- With a code, when two-factor sign-in is on.

A passkey `reauthenticate` ceremony does the same (see [Passkeys](passkeys.md)). So does a
`reauthenticate` flow through a linked identity provider (see
[Identity providers](identity-providers.md#verifying-again)).

## Security settings endpoints

Only the user themself may use these:

- `/users/{user}/security`
- `/users/{user}/totp` (with `/confirmation`)
- `/users/{user}/passkeys/{passkey}`
- `/users/{user}/recovery-codes`

A session that still owes a second factor can reach them (see
[Requiring a second factor](second-factors.md#requiring-a-second-factor)).
