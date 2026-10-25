# People and their accounts

## Signing in

| Code                       | What it means                                                                                                                  | What to do                                                                                          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------- |
| `invalidCredentials`       | Wrong username or password.                                                                                                    | Nothing, unless one account sees many. Someone may be guessing, which the sign-in rate limits slow. |
| `tooManyAttempts`          | Ten wrong passwords or codes for one account within fifteen minutes. Its second factors are locked for the rest of the window. | Wait. Repeated lockouts of one account suggest someone guessing.                                    |
| `reauthenticationRequired` | A security change needs a password or code given within `[auth] reverify_seconds`.                                             | The app asks for it.                                                                                |

## Second factors and passkeys

| Code                          | What it means                                                                                           | What to do                                                                                                   |
| ----------------------------- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `twoFactorEnrollmentRequired` | The deployment setting `require-two-factor` is on, and the account has no authenticator app or passkey. | The person adds one. Their session allows nothing else meanwhile.                                            |
| `lastSecondFactor`            | Removing the account's only second factor, where two factors are required.                              | Add another first.                                                                                           |
| `passkeysUnavailable`         | `public_url` is `http` at a host other than `localhost`, where browsers allow no passkeys.              | Serve the deployment over `https` (see [`public_url`](../configuration/address.md#the-deployments-address)). |
| `passkeyRejected`             | The passkey's answer did not verify.                                                                    | Open the deployment at its `public_url`. See [below](#passkeyrejected).                                      |
| `passkeyUnknown`              | A sign-in used a passkey no account here holds: it was removed, or its account deleted.                 | Nothing. The web client tells the browser, whose password manager may stop offering it.                      |

### `passkeyRejected`

Most often:

- the page is not at `public_url` (a different address reaching the same server), or
- `public_url`'s host changed since the passkey was made.
- An Android app signed with a certificate `[auth] android_apps` does not name ran it.

### The Android app opens the browser for passkeys

The app uses its own passkey prompt only where the deployment names it. Check, in order:

1. `curl -i https://<public_url's host>/.well-known/assetlinks.json` answers `200`,
   `application/json`, with no redirect, and names the app's package and certificate (see
   [Android apps and passkeys](../configuration/accounts.md#android-apps-and-passkeys)).
2. Google can read it: the address is reachable from the internet at port 443, with a certificate
   from a public authority.
3. The phone has a passkey provider (Google Password Manager, or another set up under Passwords and
   accounts) and runs Android 9 or later.

Google keeps what it read for a while, so a fix can take a day to reach phones. The browser still
works meanwhile.

## Identity providers

| Code                          | What it means                                                                                                   | What to do                                                                                                                      |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `identityProviderUnreachable` | The server could not read a provider's discovery document, keys, or token. The detail says which way it failed. | See [below](#identityproviderunreachable).                                                                                      |
| `identityProviderFailed`      | The provider did not sign the person in, or its answer did not verify. The detail says why.                     | A refused code or an unverified token usually means the client ID or secret is wrong: check them with `identity-providers set`. |
| `identityProviderUnavailable` | The provider is disabled or removed.                                                                            | Enable it with `identity-providers set <name> --enabled true`, if that was not meant.                                           |
| `identityProviderFlowExpired` | The sign-in took longer than ten minutes, or was finished already.                                              | The person starts again.                                                                                                        |
| `identityTaken`               | That identity at the provider is linked to another account.                                                     | The person signs in to that account, or unlinks it there first.                                                                 |
| `lastSignInMethod`            | Removing a passkey or a provider would leave the account no way in.                                             | The person adds a password, a passkey, or another provider first.                                                               |

### `identityProviderUnreachable`

- **Private or local network:** set `[identity_providers] allow_private_addresses`.
- **Certificate not trusted:** give your authority with `[identity_providers] ca_file`.
- **No discovery document, or another issuer named:** the issuer must be exactly what the provider's
  `/.well-known/openid-configuration` names, often with a path (a Keycloak realm's).
- **Redirected:** give the issuer the provider's document names, not an address that redirects to
  it.

Back-channel logouts the server refuses are answered `400` and logged with the reason
(`refused a back-channel logout`). Each refusal counts against the sending address, which is
answered `429` after 30 in quick succession (and one every ten seconds after that): a provider whose
logouts are all refused (a wrong client id, a token typed as something other than `logout+jwt`)
stops being heard until what it sends is fixed. Within a minute of a provider rotating its signing
key, a logout signed with the new key may be refused once while this server's copy of the keys
catches up; the provider's retry is taken.

## Email

| Code                        | What it means                                                                                                                                                             | What to do                                                                                                                |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `emailVerificationRequired` | The deployment setting `email-verification-required` is on, and the account's email address is not verified.                                                              | The person types the code mailed to them, or has a new one sent. If codes never arrive, see [below](#codes-never-arrive). |
| `passwordResetUnavailable`  | A password reset by email could not start. The detail says why: no account has that username, it has no verified address, or the deployment sends no mail (no `[email]`). | See [below](#passwordresetunavailable).                                                                                   |
| `passwordResetExpired`      | The reset is older than half an hour, was used, or ended after too many wrong addresses or codes.                                                                         | Start again from the sign-in screen.                                                                                      |

### Codes never arrive

1. Check the server's log for mail it gave up on.
2. Check the SMTP server's own log.
3. On a development server, the mailpit inbox shows what it sent.

### `passwordResetUnavailable`

Someone without a verified address cannot reset by email. An administrator can help them some other
way, such as deleting the account so they can register again.

## Signing in from another device

| Code                    | What it means                                                                                                                       | What to do                                                                                                                                                        |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `deviceLinkExpired`     | A sign-in code (the QR code one device shows another) can no longer be used. See [below](#devicelinkexpired).                       | Make a new code. If codes expire before anyone can scan them, check the clocks are not the problem: the code's minute is counted on the server.                   |
| `deviceLinkUsed`        | Another device scanned the sign-in code first.                                                                                      | The person makes a new code, and must not confirm the device that scanned the old one. **If they did not scan it themselves, someone photographed their screen.** |
| `deviceLinkCodeUnknown` | A typed sign-in code (the letters beside the QR code) matches no code waiting for that device. See [below](#devicelinkcodeunknown). | Check the code and type it again, or make a new one.                                                                                                              |

### `deviceLinkExpired`

A sign-in code expires when:

- it is older than a minute, unscanned;
- it was declined or already used;
- the signed-in device that confirmed it signed out or changed its password before the other device
  claimed it.

A code shown with letters beside it lasts five minutes rather than one.

### `deviceLinkCodeUnknown`

A typed code is refused alike when it:

- was mistyped;
- has expired or was cancelled;
- was shown for the other side: a signed-in device types the code of a device asking to be signed
  in, and a signed-out device the code of one offering its account.

Wrong codes are limited per address or account, and for everyone together (see
[Rate limits](../configuration/rate-limits.md#typed-sign-in-codes)). Past those, typing a code is
refused with `rateLimited` until they refill, while scanning still works.

## Registration and accounts

| Code                                                      | What it means                                                                                       | What to do                                                            |
| --------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `registrationInviteRequired`, `registrationInviteInvalid` | Registration takes an invite here, and none was given, or one that is used up, expired, or revoked. | Make one in the dashboard or with `aspen-chat-server invites create`. |
| `usernameTaken`                                           | Someone already has that name, in some mix of capitals. Usernames are unique regardless of case.    | Pick another.                                                         |

## Administration and moderation

| Code               | What it means                                                                                                                   | What to do                                                                        |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `adminRequired`    | The dashboard is only for holders of a deployment role.                                                                         | Grant one with `aspen-chat-server admin grant`, or a role in the dashboard.       |
| `deploymentBanned` | A moderator banned the account from this deployment. The detail carries the reason they gave and when the ban ends, if it does. | Holders of Ban users lift it from the dashboard's user directory (Banned filter). |
| `alreadyReported`  | The person already reported this message or profile, and the report awaits review.                                              | Review it under Reports in the dashboard.                                         |
