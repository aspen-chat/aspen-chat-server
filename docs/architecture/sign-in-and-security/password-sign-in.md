# Password sign-in

## Where it lives

| Part | Where |
| --- | --- |
| Per-username limits | `api::rate_limit::limit_sign_in`, `aspen_limits::ClientAddresses::network_key`, `RateLimiter::peek`, `note_wrong_password` |
| Hashing work limits | `app::login::password_work` |
| Unknown names and bots | `login::UNMATCHABLE_PASSWORD_HASH` |
| New password rules | `login::check_new_password` |

## The sign-in request

`POST /auth/login` answers one of:

- `{"status": "signedIn", …tokens}`.
- `{"status": "secondFactorRequired", "ticket", "methods"}`, for an account with two-factor sign-in on.

The ticket is kept in Valkey for five minutes. It is finished with `POST /auth/login/second-factor` and an authenticator code or a recovery code, or with a passkey (see [Second factors](second-factors.md) and [Passkeys](passkeys.md)).

## Per-username rate limits

There are two, both in `api::rate_limit::limit_sign_in`.

| Limit | Counts | Checked | Counted |
| --- | --- | --- | --- |
| `username` | Every attempt at a name from one client network: an IPv4 /24, an IPv6 /48 (`aspen_limits::ClientAddresses::network_key`) | Before the password is checked | Every attempt |
| `username_failures` | The name's wrong passwords from every network together | Before the password is checked, with `RateLimiter::peek`, which counts nothing | Only after a wrong password (`note_wrong_password`) |

- While `username_failures` is spent, password sign-in for the name is refused.
- Passkeys, device links, and a reset by email still reach the account then.
- An unknown name counts like a known one.

See [design notes](design-notes.md#per-username-rate-limits) for why the limits are split this way.

## Password hashing limits

Hashing and checking a password uses Argon2: 19 MiB and most of a core, for a noticeable time each.

- At most `[auth] password_hashing_threads` blocking threads do this work at once (`app::login::password_work`).
- A request that waits `password_hashing_wait_seconds` without a thread is refused with `503` `serverBusy` and `Retry-After`.
- No database connection is held while waiting for that work or doing it. Each caller reads the hash, gives its connection back, and takes one again afterwards.

**Why:** a flood of sign-ins must not hold the pool idle and refuse every other request (`database_pool_wait_seconds`). See [design notes](design-notes.md#password-hashing-limits).

## Unknown names and bots

A sign-in naming no account, or naming a bot, is checked against a fixed hash no password matches (`login::UNMATCHABLE_PASSWORD_HASH`). It takes as long as a wrong password, so it does not tell which names exist.

## New passwords

A new password, at registration, changed, or reset, is at least 8 and at most 1024 bytes (`login::check_new_password`). Otherwise it is refused with `passwordRequirementsNotMet`:

- `requirement` names `length` or `maxLength`.
- `detail` gives the limit.
