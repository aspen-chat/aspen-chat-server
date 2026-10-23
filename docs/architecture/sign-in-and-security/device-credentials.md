# Device credentials

A device credential is what a request without a session is counted by. It lets the many people
behind one address (a carrier-grade NAT, an office, an event's network) each have limits of their
own, while one machine is held to one device's, or pays a puzzle for every device more it claims to
be.

## Where it lives

| Part                               | Where                                                                                                        |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Challenges, credentials, redeeming | `app::device_credential`                                                                                     |
| The puzzle                         | `aspen_limits::puzzle`, `spec/device_puzzle_vectors.json`                                                    |
| The key                            | `app::server_secret::HmacKey`, the `devices` row of `server_secret`                                          |
| Endpoints                          | `api::device_credential`                                                                                     |
| Requiring and counting one         | `api::rate_limit::limit_requests`, `RateLimiter::requires_device`                                            |
| The client                         | `AspenClient` and `device.ts` in `@aspen/protocol`; `api/devices.ts` and `devicePuzzle.worker.ts` in the app |
| Scripts                            | `scripts/device_credential.py`; the benchmark's `bench/src/device.rs`                                        |

## Earning one

1. `POST /auth/device-challenges` answers `{challenge, difficulty, expiresAt}`.
2. The client finds a nonce for which `SHA-256(SHA-256(challenge) || nonce)` begins with
   `difficulty` zero bits. The challenge is hashed as its base64url-decoded bytes, and the nonce as
   eight big-endian bytes.
3. `POST /auth/device-credentials` with `{challenge, nonce}` answers `{credential, expiresAt}`.

|                     |                                                                                                                      |
| ------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Difficulty          | `[rate_limits] device_puzzle_bits`, 20 by default (about a million hashes), at most 32                               |
| Challenge lifetime  | Five minutes (`CHALLENGE_LIFETIME`)                                                                                  |
| Credential lifetime | Ninety days (`LIFETIME`)                                                                                             |
| Uses of a challenge | One credential. Its use is remembered in Valkey (`device-challenge:*`) until it expires.                             |
| Refusals            | `400` `deviceChallengeInvalid`, whose `detail` says why: not this deployment's, expired, already used, or not solved |

- **Stateless tokens:** a challenge and a credential are each their content and an HMAC-SHA-256
  under the deployment's device key (`HmacKey`, the `devices` secret). The challenge holds its
  version, expiry, difficulty and a random part; the credential holds its version, a random device
  id and its expiry. Each is signed under its own context, so neither can be presented as the other.
- **The difficulty is signed:** the server checks the difficulty the challenge itself names, so a
  lowered one fails the HMAC.
- **The key:** the first API server to start makes it (`HmacKey::load`), and every one reads it.
  Deleting its row and restarting the servers makes every credential and challenge invalid. Clients
  then earn new ones, a puzzle each.

## Presenting one

- **Which requests carry it:** a request without a valid session sends it as `Aspen-Device`.
- **Which routes ask for it:** every route someone may call without signing in (`Access::Anonymous`
  or `Optional`), except those `[rate_limits] device_exempt` lists, and the two above, which never
  ask (`rate_limit::DEVICE_ROUTES`). Without a valid one, such a request is refused with `401`
  `deviceCredentialRequired`.
- **Under a suspension:** while a suspension of the limits (`limits suspend`) covers a request's
  network, or every limit, the request may go without one (`RateLimiter::waives_device`), so a
  benchmark's many users from one address solve no puzzle.
- **What the built-in list exempts** (`rate_limits.toml`) is whatever isn't an Aspen client calling:
  - the health checks;
  - the passkey page and the two calls its own script makes;
  - token refresh, which phones' push handlers make;
  - the federation document, and the endpoints other deployments' servers call;
  - the unsubscribe page;
  - the web client's pages;
  - plugins' assets and capability URLs.
- **Renewal:** a credential presented with less than half its lifetime left is answered with a fresh
  one for the same device, in `Aspen-Device-Credential`. A device in use never solves the puzzle
  again.
- **CORS:** `cors_layer` allows the request header and exposes the response header.
- **OpenAPI:** each operation that asks for a credential declares the header and the `401`
  (`document_device`).

## Who a request counts as

`limit_requests` checks a request's limits in this order:

1. **`global` and `per_<param>`.**
2. **A presented session token** is looked up once, and the result is left on the request
   (`ResolvedSession`) for the session extractors.
   - A made-up or expired session token, or bot token of the signed kind, is refused by its
     signature with no lookup (see [Sessions](sessions.md#tokens)).
   - Only a bot token from before tokens were signed can be told from a made-up one by a lookup
     alone. For those, the address's `ip_rejected_tokens` is peeked at before the lookup and counted
     when the token is rejected, so they cannot keep the database busy.
3. **A valid session counts toward its user's limits** (`user`, `user_per_<param>`) and none of its
   address's.
4. **Anything else counts toward `ip`, `ip_per_<param>` and `device`.**
   - It must present a credential where the route asks for one.
   - It is refused when the route's `ip_failures` are spent (peeked at).
   - Its handler can count a failure through the `Failures` it is given.

The failures that count are:

- a wrong password or second-factor code at sign-in;
- an unknown refresh token;
- a wrong or expired password-reset code or reset;
- an unknown registration invite, whether looked up or given at registration.

People at one address mostly get these right, so the address limits can be generous, while a machine
guessing wrong is soon held.

A suspension of the address limits (`limits suspend`) also skips `device`, `ip_failures` and
`ip_rejected_tokens`, since they count by the client.

## The client

- **On demand:** `AspenClient` sends the credential it holds on every request without a session.
  When a server answers `deviceCredentialRequired`, it earns one and sends the request once more.
- **One puzzle at a time:** requests refused together share one earning. A challenge that expired
  while a slow device solved it is asked for once more.
- **The solver:** the app solves in a worker (`devicePuzzle.worker.ts`), so the page does not
  freeze. Where workers cannot run, `solvePuzzle` works on the page's thread, a slice at a time. The
  SHA-256 is plain arithmetic, not WebCrypto, whose every call is asynchronous, and it passes the
  shared vectors.
- **Storage:** the credential is kept per deployment in `localStorage` (`aspen.device.<host>`) and
  survives signing out. It says which device this is, not who uses it.
- **Native code:** the phones' push handlers make only token refreshes, which are exempt.

## Scripts and the benchmark

- **Python:** `scripts/device_credential.py` earns one credential per deployment. `stack.py`,
  `dev_federation.py` and `check_box.py` present it on requests without a token. Scripts that wait
  for a server ask `GET /health`.
- **Benchmark:** a run that suspends the limits earns none, since its suspension waives them. One
  that does not earns one for each source address of its profile, or one without them, and the users
  from an address share it.

## When access is given or taken away

A credential grants nothing: no read and no write is allowed because of one. It only decides which
budget a request without a session spends.

- **Who can observe it:** only its holder, who sent it. The server logs nothing of it and keeps
  nothing but a used challenge's random part, until the challenge expires.
- **Losing it:** nothing is open on its account. Event streams, calls and pushes all belong to
  sessions. A credential that expires, or that a new key invalidates, is answered with
  `deviceCredentialRequired`, and the client earns another.
- **Changes it makes:** none, so there is nothing to announce or to publish in a transaction.
  `scripts/check_permissions.py` gains no scenario.

## Rate limits

| Endpoint                        | Limits                                                                                         |
| ------------------------------- | ---------------------------------------------------------------------------------------------- |
| `POST /auth/device-challenges`  | 2000 an hour per address, 1000 at once                                                         |
| `POST /auth/device-credentials` | 1000 an hour per address, all at once; 6000 a minute from everyone, 3000 at once; fails closed |

Why it is built this way: [design notes](design-notes.md#device-credentials).
