# People and their accounts

## Signing in

| Code | What it means | What to do |
| --- | --- | --- |
| `invalidCredentials` | Wrong username or password. | Nothing, unless one account sees many. Someone may be guessing, which the sign-in rate limits slow. |
| `tooManyAttempts` | Ten wrong passwords or codes for one account within fifteen minutes. Its second factors are locked for the rest of the window. | Wait. Repeated lockouts of one account suggest someone guessing. |
| `reauthenticationRequired` | A security change needs a password or code given within `[auth] reverify_seconds`. | The app asks for it. |

## Second factors and passkeys

| Code | What it means | What to do |
| --- | --- | --- |
| `twoFactorEnrollmentRequired` | The deployment setting `require-two-factor` is on, and the account has no authenticator app or passkey. | The person adds one. Their session allows nothing else meanwhile. |
| `lastSecondFactor` | Removing the account's only second factor, where two factors are required. | Add another first. |
| `passkeysUnavailable` | `public_url` is `http` at a host other than `localhost`, where browsers allow no passkeys. | Serve the deployment over `https` (see [`public_url`](../configuration/address.md#the-deployments-address)). |
| `passkeyRejected` | The passkey's answer did not verify. | Open the deployment at its `public_url`. See [below](#passkeyrejected). |

### `passkeyRejected`

Most often:

- the page is not at `public_url` (a different address reaching the same server), or
- `public_url`'s host changed since the passkey was made.

## Email

| Code | What it means | What to do |
| --- | --- | --- |
| `emailVerificationRequired` | The deployment setting `email-verification-required` is on, and the account's email address is not verified. | The person types the code mailed to them, or has a new one sent. If codes never arrive, see [below](#codes-never-arrive). |
| `passwordResetUnavailable` | A password reset by email could not start. The detail says why: no account has that username, it has no verified address, or the deployment sends no mail (no `[email]`). | See [below](#passwordresetunavailable). |
| `passwordResetExpired` | The reset is older than half an hour, was used, or ended after too many wrong addresses or codes. | Start again from the sign-in screen. |

### Codes never arrive

1. Check the server's log for mail it gave up on.
2. Check the SMTP server's own log.
3. On a development server, the mailpit inbox shows what it sent.

### `passwordResetUnavailable`

Someone without a verified address cannot reset by email. An administrator can help them some
other way, such as deleting the account so they can register again.

## Signing in from another device

| Code | What it means | What to do |
| --- | --- | --- |
| `deviceLinkExpired` | A sign-in code (the QR code one device shows another) can no longer be used. See [below](#devicelinkexpired). | Make a new code. If codes expire before anyone can scan them, check the clocks are not the problem: the code's minute is counted on the server. |
| `deviceLinkUsed` | Another device scanned the sign-in code first. | The person makes a new code, and must not confirm the device that scanned the old one. **If they did not scan it themselves, someone photographed their screen.** |

### `deviceLinkExpired`

A sign-in code expires when:

- it is older than a minute, unscanned;
- it was declined or already used;
- the signed-in device that confirmed it signed out or changed its password before the other
  device claimed it.

## Registration and accounts

| Code | What it means | What to do |
| --- | --- | --- |
| `registrationInviteRequired`, `registrationInviteInvalid` | Registration takes an invite here, and none was given, or one that is used up, expired, or revoked. | Make one in the dashboard or with `aspen-chat-server invites create`. |
| `usernameTaken` | Someone already has that name, in some mix of capitals. Usernames are unique regardless of case. | Pick another. |

## Administration and moderation

| Code | What it means | What to do |
| --- | --- | --- |
| `adminRequired` | The dashboard is only for holders of a deployment role. | Grant one with `aspen-chat-server admin grant`, or a role in the dashboard. |
| `deploymentBanned` | A moderator banned the account from this deployment. The detail carries the reason they gave and when the ban ends, if it does. | Holders of Ban users lift it from the dashboard's user directory (Banned filter). |
| `alreadyReported` | The person already reported this message or profile, and the report awaits review. | Review it under Reports in the dashboard. |
