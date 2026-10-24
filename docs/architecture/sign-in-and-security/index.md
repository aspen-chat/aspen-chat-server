# Sign-in and security

How people sign in (by password, second factor, passkey, another device's QR code, or an identity
provider), how their sessions are kept and ended, which changes to their security need a recent
verification, and what a request without a session is counted by.

## Pages

- [Usernames](usernames.md): what a username may hold, and how case is ignored.
- [Password sign-in](password-sign-in.md): the sign-in request, the per-username rate limits, the
  password hashing limits, and the rules for new passwords.
- [Sessions and signing out](sessions.md): tokens, refreshing, signing out everywhere else, and what
  ending a sign-in closes.
- [Second factors](second-factors.md): authenticator apps, passkeys as a factor, recovery codes,
  attempt limits, security emails, and `require_two_factor`.
- [Reverification and security settings](reverification.md): `verified_at`,
  `POST /auth/reauthenticate`, and the security settings endpoints.
- [Passkeys](passkeys.md): WebAuthn ceremonies and the hand-off to the system browser for the
  desktop and mobile apps, the signals that keep passkey managers in step, autofill, and the Android
  apps that run ceremonies themselves.
- [Device links](device-links.md): signing in one device from another by a QR code or a typed code.
- [Identity providers](identity-providers.md): signing in through OpenID Connect providers the
  administrators chose, linking them, verifying again through them, and their back-channel logout.
- [Device credentials](device-credentials.md): the puzzle a client solves to be counted as a device
  of its own before signing in, and who a request counts as.

Why things are the way they are: [design notes](design-notes.md).

## Key files

| Part                               | Where                                                                               |
| ---------------------------------- | ----------------------------------------------------------------------------------- |
| Usernames                          | `app::user::validate_username`, `app::user::named`                                  |
| Password sign-in, tokens, sessions | `app::login`, `api::rate_limit::limit_sign_in`                                      |
| Second factors, attempt limits     | `app::two_factor`                                                                   |
| Passkeys                           | `app::passkey`, `api::passkey_page`, `api::asset_links`                             |
| Device links                       | `app::device_link`                                                                  |
| Identity providers                 | `app::identity_provider`, `api::identity_provider`                                  |
| Device credentials                 | `app::device_credential`, `aspen_limits::puzzle`, `api::rate_limit::limit_requests` |
| Email addresses and password reset | see [Email](../email/index.md)                                                      |
