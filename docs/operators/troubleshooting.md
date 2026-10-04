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
| `twoFactorEnrollmentRequired` | `[auth] require_two_factor` is on and the account has no authenticator app or passkey. | The person adds one; their session allows nothing else meanwhile. |
| `lastSecondFactor` | Removing the account's only second factor, where two factors are required. | Add another first. |
| `reauthenticationRequired` | A security change needs a password or code given within `[auth] reverify_seconds`. | The app asks for it. |
| `passkeysUnavailable` | `[auth.passkeys]` is not set. | Set it (see [Configuration](configuration.md#authpasskeys)) to offer passkeys. |
| `passkeyRejected` | The passkey's answer did not verify. Most often the page's origin is not in `[auth.passkeys] origins`, or `rp_id` changed since the passkey was made. | Check both settings against the address in the browser. |
| `registrationInviteRequired`, `registrationInviteInvalid` | Registration takes an invite here, and none, or one that is used up, expired, or revoked, was given. | Make one in the dashboard or with `aspen-chat-server invites create`. |
| `usernameTaken` | Someone already has that name, in some mix of capitals: usernames are unique regardless of case. | Pick another. |
| `adminRequired` | The dashboard is only for holders of a deployment role. | Grant one with `aspen-chat-server admin grant`, or a role in the dashboard. |
| `deploymentBanned` | A moderator banned the account from this deployment; the detail carries the reason they gave and when the ban ends, if it does. | Holders of Ban users lift it from the dashboard's user directory (Banned filter). |
| `alreadyReported` | The person already reported this message or profile, and the report awaits review. | Review it under Reports in the dashboard. |

### Load

| Code | What it means | What to do |
| --- | --- | --- |
| `rateLimited` | Too many requests to one endpoint; `Retry-After` says for how long. | If many people share one address (an office, a school, a load test), check `[rate_limits] trusted_proxies` first: behind a proxy that is not listed, everyone counts as the proxy. `aspen-chat-server limits suspend --scope networks --network <cidr> --reason …` lifts the per-address limits for a network for a while. |
| `serverBusy` | More sign-ins than `[auth] password_hashing_threads` can check within `password_hashing_wait_seconds`. | Brief bursts are expected (after an outage, everyone signs in again). If it lasts, add API servers or CPU. |
| `internal` | Something failed on the server. | The log has an `ERROR` line for each, with the cause. Most are the database, NATS, Valkey, or storage being unreachable. |

### Federation

| Code | What it means | What to do |
| --- | --- | --- |
| `deploymentUnreachable` | This deployment could not read another's document. The detail says why: its name has no address, it answered only on private addresses, it refused the connection, it timed out, its certificate is expired, not yet valid, for another name, or not from a public authority, it redirected, or it served something that is not an Aspen document. | Each detail says whose to fix. To check by hand: `curl -v https://<domain>/.well-known/aspen`. |
| `federationRefused` | A gate does not let this crossing happen: yours or theirs, for this deployment, for this kind of account; or federation is off (`[federation] domain` unset); or the two speak no protocol version in common; or the person is banned here. The detail says which. | Change the gate or a list if you mean to allow it. |
| `assertionInvalid` | A statement from another deployment was refused: its key changed without a handover, the signature does not match, it was meant for another deployment, it expired or is from the future (a clock is wrong), or it was used before. The detail says which. | For a changed key, see [Keys](federation.md#keys). For clocks, run NTP on every server. |
| `strongerSignInRequired` | This deployment requires two factors, and the visitor signed in at home with a password alone. | They sign in at home with a second factor or a passkey. |

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
| 4403 | Two factors are required and the account has none. |
| 4408 | No `identify` within ten seconds. Usually a very slow connection. |
| 4410 | The account was banned from this deployment. The app signs out and says why. |

If connections drop every minute or so, a proxy between clients and the server is closing idle
WebSockets: the server pings every thirty seconds, so any idle timeout must be longer than that.
If they never open, check that the proxy passes `Upgrade` and `Connection` through for
`/api/v1/events`.

## Symptoms

**The web client says it cannot reach the server.** Open `https://<your domain>/api/v1/auth/methods`
in a browser: it should answer JSON. If the web client is served from another origin, that
origin must be in `[cors] allowed_origins`.

**Uploads fail.** The browser uploads straight to `[media.s3] public_endpoint`. It must be
reachable from the client, over HTTPS when the page is, and allow the page's origin by CORS for
`PUT`. The browser's developer tools show the refused request.

**Pictures do not load.** They load from `[media.s3] public_base_url`, which must serve the bucket
without credentials.

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
`failure_window_seconds`. Fix the cause (usually its TLS proxy or its ports), then enable it
again in the dashboard.

**The server will not start.** It says why on standard error: a setting it cannot read, a rate
limit naming an endpoint that does not exist, federation gates open without a `domain`, or a
service it cannot reach.

**`aspen-migrate up` stops at "usernames that differ only by case".** Usernames are unique
regardless of case, so people can sign in however they type theirs, and the migration that makes
them so names the accounts that clash. Ask all but one of each group to change their username
under their profile, then run it again.

**Everyone is signed out after an upgrade.** It should not happen: sessions are in the database.
Check that the new servers point at the same `database_url`.
