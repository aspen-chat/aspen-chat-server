# Sessions and signing out

Code: `app::login` (`token_digest`, `sign_in_id`, `try_token_refresh`, `revoke_other_sign_ins`, `end_other_sign_ins`).

## Tokens

- Refresh and session tokens are 256 random bits.
- They are kept only as their SHA-256 in hex (`login::token_digest`), as bot tokens are. A presented token is looked up by its digest, so a copy of the database signs no one in.
- A sign-in's id (`login::sign_in_id`) is the first half of its refresh token's digest.
- A refresh token is not rotated when used. It stands for its sign-in until it expires (a year) or the sign-in ends.

## Refreshing

`POST /auth/token-refresh` (`login::try_token_refresh`) gives no session token once its account is deleted or banned from the deployment. It answers `invalidToken`, as an ended sign-in's does, so the app signs out. Signing in again tells of a ban.

## What signs out other sign-ins

| Change | What it ends | Code |
| --- | --- | --- |
| Adding the first second factor | Every other sign-in | `login::revoke_other_sign_ins` |
| Changing the password | Every other session | |
| `DELETE /users/{user}/sign-ins` (signing out everywhere else) | Every other sign-in. Needs a recently verified session, and publishes `signInsEnded` inside its transaction | `login::end_other_sign_ins` |

Changing or resetting the password, and signing out everywhere else, also end every plugin capability URL of the account in the same transaction (see [Plugins](../plugins/index.md)).

## What ending a sign-in does

Signing out, or being signed out by one of the changes above:

- Closes that sign-in's open event streams (`signInsEnded`; see [Event routing](../event-routing/index.md)).
- Takes it out of any call it joined. Its join token names the sign-in (see [Voice](../voice/index.md)).
- Stops its phone being woken.

## Email addresses

An account may also hold an email address. Through it:

- A deployment can require the address verified before the account is used. Such a session is held like a session owing a second factor, with close code 4428.
- A forgotten password is reset.

Both are described in [Email](../email/index.md).
