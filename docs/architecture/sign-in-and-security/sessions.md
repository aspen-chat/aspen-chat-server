# Sessions and signing out

Code: `app::login` (`token_digest`, `sign_in_id`, `try_token_refresh`, `revoke_other_sign_ins`,
`end_other_sign_ins`), `app::signed_token`.

## Tokens

- A refresh token is 256 random bits.
- A session token is 256 random bits and its expiry, signed with an HMAC under the deployment's
  token key (`signed_token::session_token`, the `tokens` row of `server_secret`).
- Both are kept only as their SHA-256 in hex (`login::token_digest`), as bot tokens are. A presented
  token is looked up by its digest, so a copy of the database signs no one in.
- **Refused without a lookup:** a session token this deployment did not sign, or one past its
  expiry. So made-up tokens cost the server nothing, and need no limit on their address that
  everyone signed in behind it would share.
- **Remembered once rejected:** a signed one the lookup rejects (its sign-in ended, its account
  deleted or banned) is remembered by each API server (`signed_token::Rejected`, at most 100000, for
  a session token's lifetime), so presenting it again costs no lookup either. An account whose ban
  is lifted has its app refresh past that, with its refresh token.
- **Tokens from before session tokens were signed** are refused, and the apps refresh them on that
  `401`.
- A sign-in's id (`login::sign_in_id`) is the first half of its refresh token's digest.
- A refresh token is not rotated when used. It stands for its sign-in until it expires (a year) or
  the sign-in ends.
- A sign-in made through an identity provider records the provider and the provider's session id
  (`identity_provider`, `identity_provider_session`), which its back-channel logout ends it by (see
  [Identity providers](identity-providers.md#back-channel-logout)). One given by a device link
  copies them from the sign-in that gave it.

## Refreshing

`POST /auth/token-refresh` (`login::try_token_refresh`) gives no session token once its account is
deleted or banned from the deployment. It answers `invalidToken`, as an ended sign-in's does, so the
app signs out. Signing in again tells of a ban.

## What signs out other sign-ins

| Change                                                        | What it ends                                                                                                | Code                           |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------ |
| Adding the first second factor                                | Every other sign-in                                                                                         | `login::revoke_other_sign_ins` |
| Changing the password                                         | Every other session                                                                                         |                                |
| `DELETE /users/{user}/sign-ins` (signing out everywhere else) | Every other sign-in. Needs a recently verified session, and publishes `signInsEnded` inside its transaction | `login::end_other_sign_ins`    |

Changing or resetting the password, and signing out everywhere else, also end every plugin
capability URL of the account in the same transaction (see [Plugins](../plugins/index.md)).

## What ending a sign-in does

Signing out, or being signed out by one of the changes above:

- Closes that sign-in's open event streams (`signInsEnded`; see
  [Event routing](../event-routing/index.md)).
- Takes it out of any call it joined. Its join token names the sign-in (see
  [Voice](../voice/index.md)).
- Stops its phone being woken.

## Email addresses

An account may also hold an email address. Through it:

- A deployment can require the address verified before the account is used. Such a session is held
  like a session owing a second factor, with close code 4428.
- A forgotten password is reset.

Both are described in [Email](../email/index.md).
