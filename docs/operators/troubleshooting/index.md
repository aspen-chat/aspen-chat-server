# Troubleshooting

Every error Aspen shows people is a Problem with:

- a `code`, which tells you which entry below applies;
- a short `title`;
- a `detail` that says what went wrong and what to do, in the reader's language.

When someone reports an error, ask for the text they saw, or find the request in the server's
log, which is always in English.

## Logs

The server logs to standard error. `ASPEN_LOG` sets how much, as `RUST_LOG` does:

- `ASPEN_LOG=info`
- `ASPEN_LOG=info,aspen_chat_server=debug` for more from Aspen itself.

## Pages

- [People and their accounts](accounts.md): sign-in, second factors, passkeys, registration, and
  bans.
- [Load](load.md): rate limits, upload quotas, and a busy server.
- [Federation](federation.md): reaching and trusting other deployments.
- [Plugins](plugins.md): plugins refusing messages or failing.
- [The live connection](live-connection.md): the event stream's close codes, and connections
  that drop or never open.
- [Web client and storage](web-and-storage.md): the web client cannot reach the server, uploads,
  pictures, and videos.
- [Calls](calls.md): nobody can join, no sound, Silent or unregistered voice servers, repaired
  call records, and suspended voice servers.
- [Phones](phones.md): phones that are not woken.
- [The server](server.md): starting, background jobs, settings, migrations, and upgrades.

## Every error code

| Code | Where it is explained |
| --- | --- |
| `invalidCredentials` | [People and their accounts](accounts.md) |
| `tooManyAttempts` | [People and their accounts](accounts.md) |
| `twoFactorEnrollmentRequired` | [People and their accounts](accounts.md) |
| `emailVerificationRequired` | [People and their accounts](accounts.md) |
| `passwordResetUnavailable` | [People and their accounts](accounts.md) |
| `passwordResetExpired` | [People and their accounts](accounts.md) |
| `lastSecondFactor` | [People and their accounts](accounts.md) |
| `reauthenticationRequired` | [People and their accounts](accounts.md) |
| `passkeysUnavailable` | [People and their accounts](accounts.md) |
| `passkeyRejected` | [People and their accounts](accounts.md) |
| `deviceLinkExpired` | [People and their accounts](accounts.md) |
| `deviceLinkUsed` | [People and their accounts](accounts.md) |
| `registrationInviteRequired` | [People and their accounts](accounts.md) |
| `registrationInviteInvalid` | [People and their accounts](accounts.md) |
| `usernameTaken` | [People and their accounts](accounts.md) |
| `adminRequired` | [People and their accounts](accounts.md) |
| `deploymentBanned` | [People and their accounts](accounts.md) |
| `alreadyReported` | [People and their accounts](accounts.md) |
| `rateLimited` | [Load](load.md#ratelimited) |
| `uploadQuotaExceeded` | [Load](load.md#uploadquotaexceeded) |
| `serverBusy` | [Load](load.md#serverbusy) |
| `internal` | [Load](load.md#internal) |
| `deploymentUnreachable` | [Federation](federation.md#deploymentunreachable) |
| `federationRefused` | [Federation](federation.md#federationrefused) |
| `assertionInvalid` | [Federation](federation.md#assertioninvalid) |
| `strongerSignInRequired` | [Federation](federation.md#strongersigninrequired) |
| `pluginRefused` | [Plugins](plugins.md#pluginrefused) |
| `pluginUnavailable` | [Plugins](plugins.md#pluginunavailable) |
| Any other code | [Codes that need no operator](#codes-that-need-no-operator) |

## Codes that need no operator

These answer what someone asked for, and their details say what to change:

`badRequest`, `validation`, `notFound`, `forbidden`, `conflict`, `unauthorized`,
`invalidToken`, `verificationFailed`, `pollClosed`, `inviteCodeTaken`, `blocked`,
`oldPasswordIncorrect`, `passwordRequirementsNotMet`.
