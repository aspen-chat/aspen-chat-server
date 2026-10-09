# Sign-in and security

How people sign in (by password, second factor, passkey, or another device's QR code), how their sessions are kept and ended, and which changes to their security need a recent verification.

## Pages

- [Usernames](usernames.md): what a username may hold, and how case is ignored.
- [Password sign-in](password-sign-in.md): the sign-in request, the per-username rate limits, the password hashing limits, and the rules for new passwords.
- [Sessions and signing out](sessions.md): tokens, refreshing, signing out everywhere else, and what ending a sign-in closes.
- [Second factors](second-factors.md): authenticator apps, passkeys as a factor, recovery codes, attempt limits, security emails, and `require_two_factor`.
- [Reverification and security settings](reverification.md): `verified_at`, `POST /auth/reauthenticate`, and the security settings endpoints.
- [Passkeys](passkeys.md): WebAuthn ceremonies and the hand-off to the system browser for the desktop and mobile apps.
- [Device links](device-links.md): signing in one device from another by a QR code.

Why things are the way they are: [design notes](design-notes.md).

## Key files

| Part | Where |
| --- | --- |
| Usernames | `app::user::validate_username`, `app::user::named` |
| Password sign-in, tokens, sessions | `app::login`, `api::rate_limit::limit_sign_in` |
| Second factors, attempt limits | `app::two_factor` |
| Passkeys | `app::passkey`, `api::passkey_page` |
| Device links | `app::device_link` |
| Email addresses and password reset | see [Email](../email/index.md) |
