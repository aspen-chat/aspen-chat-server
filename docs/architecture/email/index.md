# Email

Email is optional, twice over. A deployment sends mail only when `[email]` is in `aspen.toml`. And an account need not have an address unless the deployment's administrators set `email_required`.

## Parts

| Part | Where |
| --- | --- |
| Mailer, made once per API server | `app::email::Mailer`, `GlobalServerContext::mailer` |
| Mail template (HTML and plain text) | `server/templates/email/letter.html`, `letter.txt`, compiled by `askama` in `aspen_email_templates` |
| Addresses, verification, preferences | `app::email`, table `user_email` |
| Password reset | `app::email::reset` |
| Queue and senders | `app::email::outbox`, the `sendEmail` job (see [Jobs](../jobs/index.md)) |
| Daily digest | `app::email::digest` |
| Newsletter | `app::email::newsletter` |
| Unsubscribe page | `api::email::unsubscribe_page` |

## Without `[email]`

- Accounts have no address.
- `GET /deployment` says `email.available: false`.
- The deployment settings that need mail cannot be turned on (`emailSettingNeedsMail`). This is checked in `app::deployment_settings::update`, from the dashboard and the terminal alike.

## How mail is written

- Sent by `lettre` over SMTP.
- Written from one template as both HTML and plain text. `askama` escapes the HTML version.
- In the language the account last used: `user_email.locale`, the request's negotiated locale when the address or its preferences were last written.

## Pages

- [Addresses](addresses.md): the one address per account, giving and removing it, preferences, and the shown address.
- [Verification](verification.md): verification codes, security notices, and `email_verification_required`.
- [Password reset](password-reset.md): the three unauthenticated steps of resetting a password by email.
- [The outbox](outbox.md): queueing mail as jobs, their classes, the senders, the sending rate, and retries.
- [The daily digest](digest.md): when digests are due and what they hold.
- [The newsletter and unsubscribing](newsletter.md): posts, sending them in batches, and unsubscribe links.
- [Access](access.md): account deletion and the revocation checklist.

[Design notes](design-notes.md) hold the reasons behind these choices.
