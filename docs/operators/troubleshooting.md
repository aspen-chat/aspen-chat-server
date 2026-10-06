# Troubleshooting

Every error Aspen shows people is a Problem with a `code`, a short `title`, and a `detail` that
says what went wrong and what to do, written in the reader's language. When someone reports an
error, ask for the text they saw, or find the request in the server's log, which is always in
English. The code tells you which of the entries below applies.

The server logs to standard error. `ASPEN_LOG` sets how much, as `RUST_LOG` does
(`ASPEN_LOG=info`, or `ASPEN_LOG=info,aspen_chat_server=debug` for more from Aspen itself).

## By error code

### People and their accounts

| Code | What it means | What to do |
| --- | --- | --- |
| `invalidCredentials` | Wrong username or password. | Nothing, unless one account sees many: someone may be guessing, which the sign-in rate limits slow. |
| `tooManyAttempts` | Ten wrong passwords or codes for one account within fifteen minutes; its second factors are locked for the rest of the window. | Wait. Repeated lockouts of one account suggest someone guessing. |
| `twoFactorEnrollmentRequired` | The deployment setting `require-two-factor` is on and the account has no authenticator app or passkey. | The person adds one; their session allows nothing else meanwhile. |
| `emailVerificationRequired` | The deployment setting `email-verification-required` is on and the account's email address is not verified. | The person types the code mailed to them, or has a new one sent. If codes never arrive, check the server's log for mail it gave up on and the SMTP server's own log; the mailpit inbox shows what a development server sent. |
| `passwordResetUnavailable` | A password reset by email could not start: no account has that username, it has no verified address, or the deployment sends no mail (no `[email]`); the detail says which. | Someone without a verified address cannot reset by email. An administrator can help them some other way, such as deleting the account so they can register again. |
| `emailMismatch` | During a password reset, the address typed is not the account's. Five wrong addresses end the reset. | The person checks it; the reset showed them its first three characters and its domain. |
| `passwordResetExpired` | The reset is older than half an hour, was used, or ended after too many wrong addresses or codes. | Start again from the sign-in screen. |
| `lastSecondFactor` | Removing the account's only second factor, where two factors are required. | Add another first. |
| `reauthenticationRequired` | A security change needs a password or code given within `[auth] reverify_seconds`. | The app asks for it. |
| `passkeysUnavailable` | `public_url` is `http` at a host other than `localhost`, where browsers allow no passkeys. | Serve the deployment over `https` (see [Configuration](configuration.md#the-deployments-address)). |
| `passkeyRejected` | The passkey's answer did not verify. Most often the page is not at `public_url` (a different address reaching the same server), or `public_url`'s host changed since the passkey was made. | Open the deployment at its `public_url`. |
| `deviceLinkExpired` | A sign-in code (the QR code one device shows another) is older than a minute unscanned, was declined or already used, or the signed-in device that confirmed it signed out or changed its password before the other claimed it. | Make a new code. If codes expire before anyone can scan them, check the clocks are not the problem: the code's minute is counted on the server. |
| `deviceLinkUsed` | Another device scanned the sign-in code first. | The person makes a new code and must not confirm the device that scanned the old one. If they did not scan it themselves, someone photographed their screen. |
| `registrationInviteRequired`, `registrationInviteInvalid` | Registration takes an invite here, and none, or one that is used up, expired, or revoked, was given. | Make one in the dashboard or with `aspen-chat-server invites create`. |
| `usernameTaken` | Someone already has that name, in some mix of capitals: usernames are unique regardless of case. | Pick another. |
| `adminRequired` | The dashboard is only for holders of a deployment role. | Grant one with `aspen-chat-server admin grant`, or a role in the dashboard. |
| `deploymentBanned` | A moderator banned the account from this deployment; the detail carries the reason they gave and when the ban ends, if it does. | Holders of Ban users lift it from the dashboard's user directory (Banned filter). |
| `alreadyReported` | The person already reported this message or profile, and the report awaits review. | Review it under Reports in the dashboard. |

### Load

| Code | What it means | What to do |
| --- | --- | --- |
| `rateLimited` | Too many requests to one endpoint; `Retry-After` says for how long. | If many people share one address (an office, a school, a load test), check `[rate_limits] trusted_proxies` first: behind a proxy that is not listed, everyone counts as the proxy. `aspen-chat-server limits suspend --scope networks --network <cidr> --reason …` lifts the per-address limits for a network for a while. |
| `serverBusy` | More sign-ins than `[auth] password_hashing_threads` can check within `password_hashing_wait_seconds`, or every database connection stayed taken for `database_pool_wait_seconds` (the server logs "no database connection came free"). | Brief bursts are expected (after an outage, everyone signs in again). If sign-ins cause it and it lasts, add API servers or CPU. If database connections do, check NATS first: every write holds its connection until NATS acknowledges its event. Then raise `database_pool_size` within what PostgreSQL's `max_connections` allows, or add API servers. |
| `internal` | Something failed on the server. | The log has an `ERROR` line for each, with the cause. Most are the database, NATS, Valkey, or storage being unreachable. |

### Federation

| Code | What it means | What to do |
| --- | --- | --- |
| `deploymentUnreachable` | This deployment could not read another's document. The detail says why: its name has no address, it answered only on private addresses, it refused the connection, it timed out, its certificate is expired, not yet valid, for another name, or not from a public authority, it redirected, or it served something that is not an Aspen document. | Each detail says whose to fix. To check by hand: `curl -v https://<domain>/.well-known/aspen`. |
| `federationRefused` | A gate does not let this crossing happen: yours or theirs, for this deployment, for this kind of account; or federation is off (an `http` `public_url`); or the two speak no protocol version in common; or the person is banned here. The detail says which. | Change the gate or a list if you mean to allow it. |
| `assertionInvalid` | A statement from another deployment was refused: its key changed without a handover, the signature does not match, it was meant for another deployment, it expired or is from the future (a clock is wrong), or it was used before. The detail says which. | For a changed key, see [Keys](federation.md#keys). For clocks, run NTP on every server. |
| `strongerSignInRequired` | This deployment requires two factors, and the visitor signed in at home with a password alone. | They sign in at home with a second factor or a passkey. |

### Plugins

| Code | What it means | What to do |
| --- | --- | --- |
| `pluginRefused` | A plugin refused a message before it was saved; the detail is the plugin's reason, in the reader's language. | Nothing, unless the plugin is wrong: its community settings (or yours, `plugins show <id>`) decide what it refuses. |
| `pluginUnavailable` | A plugin that must decide messages could not (it failed, or ran out of time), and its manifest says to refuse them then; or a plugin's route could not answer. The detail names the plugin. | The log has a line for each failure, with the plugin's id. Raise `[plugins] intercept_millis` if it only runs out of time; `plugins disable <id>` stops it meanwhile. |

### Everything else

`badRequest`, `validation`, `notFound`, `forbidden`, `conflict`, `unauthorized`,
`invalidToken`, `verificationFailed`, `pollClosed`, `inviteCodeTaken`, `blocked`,
`oldPasswordIncorrect`, and `passwordRequirementsNotMet` answer what someone asked for, and their
details say what to change. They need no operator.

## The live connection

The app keeps a WebSocket open to `/api/v1/events`. When the server closes it on purpose it
uses one of these codes, after a message saying why:

| Close code | Why |
| --- | --- |
| 4400 | The first message was not a well-formed `identify`. An outdated client, or a proxy mangling the connection. |
| 4401 | The session is not valid: expired, signed out elsewhere, or revoked, before the stream opened or while it was open (signing out, a password or second factor change, a deleted account). The app signs in again. |
| 4403 | Two factors are required and the account has none, when the stream opens or when the requirement is turned on while it is open. |
| 4408 | No `identify` within ten seconds. Usually a very slow connection. |
| 4410 | The account was banned from this deployment. The app signs out and says why. |

If connections drop every minute or so, a proxy between clients and the server is closing idle
WebSockets: the server pings every thirty seconds, so any idle timeout must be longer than that.
If they never open, check that the proxy passes `Upgrade` and `Connection` through for
`/api/v1/events`.

## Symptoms

**The web client says it cannot reach the server.** Open `https://<your domain>/api/v1/auth/methods`
in a browser: it should answer JSON. If it answers a page instead, your proxy sends `/api/` to
something other than the API servers.

**Uploads fail.** The browser uploads straight to `[media.s3] public_endpoint`. It must be
reachable from the client, over HTTPS when the page is, and allow the page's origin by CORS for
`PUT`. The browser's developer tools show the refused request.

**Pictures do not load.** They load from `[media.s3] public_base_url`, which must serve the bucket
without credentials.

**Videos show as downloads, without a poster.** No server that makes previews can run `ffmpeg`
and `ffprobe`; each says so in its log as it starts ("makes previews of pictures only"). Install
them, or name them in [`[media.previews]`](configuration.md#mediapreviews). An HDR video, or one
larger than `max_video_bytes`, has no poster by design.

**Messages with a picture take twenty seconds to appear.** No server makes previews (`[media.previews]
make` is off everywhere, or every maker is stuck), so each message waits out its hold. The
`attachment_preview_job` table shows what is waiting.

**Nobody can join a call** ("No voice server can take a call right now"). No voice server is
enabled, has room, and reported within `[voice] offer_silence_seconds`. The dashboard's Server fleet
tab shows each voice server's last report. Check that the voice server is running, reaches
NATS with the same token, and has the `id` the registry gave it.

**The log warns that a voice server's snapshot repaired the record of a call**, or that a voice
server no longer holds a call recorded on it. Some of that voice server's reports never reached
an API server: NATS was unreachable or restarted, or a report kept failing (an `ERROR` line says
which). The record is right again, and someone who appeared missing from, or stuck in, a call
for up to a minute was that. Warnings that keep coming mean the voice server's link to NATS
keeps dropping.

**People join a call but hear nothing.** Signalling works but media does not flow: the media
ports (`[rtc] min_port` to `max_port`, UDP and TCP) are closed, or `[rtc] announced_address` is
not the address clients can reach (behind NAT it must be the public one).

**Phones are not woken.** The app registers each phone when it signs in, and the server then
calls the phone's relay over HTTPS. Check that `[push] enabled` is on and that the server can
reach the relay the phones' app names over HTTPS (its address appears in the log beside any
push that failed). The log warns
of every push a relay refuses: `quotaExhausted` means the deployment has sent more pushes this
month than its tier with that relay allows, and pushes resume next month or when its tier is
raised. Nobody is woken for a message while they are using Aspen on another device, in a muted
channel, or by someone they blocked.

**A voice server was disabled.** `failure_threshold` people failed to start a call on it within
`failure_window_seconds`, each of them sent to it by a join offer moments before (a report from
anyone else, or from a bot, does not count). Fix the cause (usually its TLS proxy or its ports), then enable it
again in the dashboard.

**The server will not start.** It says why on standard error: a setting it cannot read, no
built web client in `[web_client] dir`, a rate limit naming an endpoint that does not exist, a
`public_url` whose host is not the domain this deployment is already known by (or an `http` one,
once it has a domain), or a service it cannot reach.

**A setting changed but a server did not follow.** Every API server watches NATS for changes to
the [deployment settings](configuration.md#deployment-settings) and reads them from the database
as they commit. One that cannot reach NATS logs `following the deployment's settings failed` and
tries again every five seconds, reading the settings afresh when it reconnects.

**`aspen-migrate up` stops at "usernames that differ only by case".** Usernames are unique
regardless of case, so people can sign in however they type theirs, and the migration that makes
them so names the accounts that clash. Ask all but one of each group to change their username
under their profile, then run it again.

**Everyone is signed out after an upgrade.** It should not happen: sessions are in the database.
Check that the new servers point at the same `database_url`.
