# Sign-in and security: design notes

Why sign-in and security work as they do. The how is in the [main pages](index.md).

## Usernames

See [Usernames](usernames.md).

- **No `@` in a username.** It sets a tag apart.
- **No whitespace, control, or format characters.** By these, two names that look alike could differ. Format characters (zero-width spaces and joiners, direction marks and isolates, the soft hyphen, tags) are invisible.
- **Names must be NFC.** So one name has one spelling.

## Per-username rate limits

See [Password sign-in](password-sign-in.md#per-username-rate-limits).

- **`username` counts per client network, not globally.** So one network's guesses cannot lock the owner out from anywhere else.
- **`username_failures` is peeked before the check and counted only after a wrong password.** So the owner's right passwords never spend it.
- **Passkeys, device links, and a reset by email stay open while `username_failures` is spent.** The owner keeps a way in that a guesser cannot close.
- **An unknown name counts like a known one.** So the limits tell nothing of which names exist. For the same reason, a sign-in naming no account or a bot is checked against `login::UNMATCHABLE_PASSWORD_HASH`, taking as long as a wrong password.

## Password hashing limits

See [Password sign-in](password-sign-in.md#password-hashing-limits).

- **Argon2 work runs on a bounded number of threads, and a request that waits too long is refused with `serverBusy`.** A burst of sign-ins then costs a bounded amount of memory and leaves the rest of the server its CPU. It sheds what it cannot serve soon rather than slowing everyone down.
- **No database connection is held while waiting for or doing hashing work.** Otherwise a flood of sign-ins could hold the pool idle and refuse every other request (`database_pool_wait_seconds`).

## Tokens

See [Sessions and signing out](sessions.md#tokens).

- **Tokens are kept only as their SHA-256 digests.** A copy of the database signs no one in.
- **A refresh token is not rotated when used.** It stands for its sign-in until it expires or the sign-in ends.
- **A refresh token of a deleted or banned account answers `invalidToken`, as an ended sign-in's does.** So the app signs out, and signing in again tells of a ban.

## Attempt limits

See [Second factors](second-factors.md#attempt-limits).

- **A daily window as well as a quarter-hour one, independent of how the quarter hours fall.** So the codes cannot be guessed slowly either.
- **Each attempt is counted before it is checked, in one script.** So attempts sent all at once are checked no more than ten times.
- **The count is per user.** That lets someone who knows the password keep it reached and lock the owner's authenticator codes and password re-verification for a day.
- **Recovery codes and passkeys are checked outside the limits, and a reset by email does not use them.** A recovery code is 50 random bits, one of ten, beyond guessing at any pace the endpoints allow. So the owner always has a way in that the lock does not close.

## Passkeys

See [Passkeys](passkeys.md).

- **A discoverable credential is required at registration.** So passkey sign-in needs no username.
- **The apps hand ceremonies to a page on the deployment.** A browser offers a passkey only to pages under `rp_id`, which the desktop and mobile apps are not.
- **The return address is a loopback port or the `aspen:` scheme, never a web page.** The web client runs ceremonies in its own page, and a web page that forwarded its query would hand the return code on.
- **The ceremony id travels in the URL fragment.** So it reaches no server log.

### The return code

- **The claim needs a one-time return code as well as the verifier.** The page's link is a real link on the deployment's own origin, so whoever starts a ceremony can send it to someone else. The verifier alone would let them claim what that person's passkey approved: a session in their account for a `signIn`, or a verification of a stolen session for a `reauthenticate`.
- Every return address the server accepts leads back to the device whose browser ran the page (its own loopback or its own app). So the code never reaches someone who sent the link elsewhere, and a ceremony is claimed only where it was run.
- The page also tells whoever opens it to continue only for a request they made in the app on that device.

## Device links

See [Device links](device-links.md).

- **A signed-in phone may not scan an offer.** Claiming would replace its sign-in.
- **One scan takes the link (`SET NX`), and the loser gets `deviceLinkUsed`.** This tells someone whose screen was photographed.
- **Cancelling uses `SET XX` for scans and approvals in flight.** So a cancelled code stays cancelled.
- **Only the receiver can claim, with its verifier.** A giver approving a stranger's request, or a stranger holding a photographed code, gets no session.
- **Nothing about the devices is looked up or recorded beyond the name each gives itself.**
- **The new sign-in copies the giver's `method` and `verified_at`.** So it is no stronger than the giver's.
- **Giving a sign-in takes a recent verification.** Without it a stolen session could mint a sign-in of its own on another device, outliving the stolen one.
- **Starts are limited globally as well as per address.** Each start keeps state in Valkey until it expires.
- **Nothing is announced; both sides poll.** A link lives for a minute and has no event.

## Device credentials

See [Device credentials](device-credentials.md).

- **People behind one address are told apart by something they hold, not by where they are.** A carrier-grade NAT puts thousands of people behind one IPv4 address, and an event's network everyone in the room. Counted by address alone, they would share one person's limits. Counted by device, each has their own, and the address keeps only a ceiling high enough for a crowd.
- **A credential costs a puzzle, once.** A credential that cost nothing would let one machine be as many devices as it asked for. The puzzle makes every device one machine claims cost it the same work a person pays once per deployment. It is not meant to stop a determined attacker, whose hardware solves it far faster than a phone, but to make each device cost something.
- **SHA-256 rather than a memory-hard puzzle.** It needs no dependency on either side, and the server checks a solution with two hashes, so made-up solutions cost it nothing. A memory-hard puzzle would narrow what a GPU gains, but every check of a solution would then cost the server as much as one try costs the client.
- **Challenges and credentials are stateless.** Every API server checks them with the shared key and asks no one, so presenting a credential adds no lookup to a request. Only a challenge's use is remembered, until it expires, so that one solution earns one credential.
- **Ninety days, renewed past half its life.** A device in use never solves the puzzle again, and one hoarded or copied ages out.
- **Signed-in requests count by user alone.** A session already says who is asking, and it is something an address's people each hold. Counting their requests against their address too would hold everyone behind a NAT to one budget as soon as many were signed in.
- **Session and bot tokens are signed, so a made-up one is refused without the database.** Looking up a token takes the database, and signed-in requests are not counted by their address before their token is read. A limit on an address's rejected tokens, checked before each lookup, would hold back everyone signed in behind that address while one machine there sent made-up tokens. A token's signature tells a made-up one apart with no lookup, so nothing needs that limit. Only bot tokens from before signing keep it, since only a lookup tells them from made-up ones, and a bot cannot refresh to a signed one as an app does.
- **A signed token the database rejected is remembered.** Otherwise one machine could present its own ended session over and over, and each time cost a lookup.
- **Failures are counted, not just attempts.** A crowd mostly knows its passwords and its invites, and a machine guessing does not. Counting what proves wrong lets an address's attempts be generous while its wrong guesses are few.
- **Token refresh and the passkey page are exempt.** A phone's push handler refreshes its session without the app running, and cannot solve the puzzle. A refresh token is a secret only its holder knows, so a valid one is counted only by the address's ceiling, and unknown ones as failures. The passkey page is a page the system browser opens, not an Aspen client. Its calls each name a ceremony only its starter knows.
- **The client earns a credential when asked, not ahead of time.** Then a server that asks for none, or a stub in a test, costs it nothing, and the first refusal costs one round trip more.
- **A credential survives signing out.** It names the device, not the person, and a person signing out and in again would otherwise pay the puzzle each time.

