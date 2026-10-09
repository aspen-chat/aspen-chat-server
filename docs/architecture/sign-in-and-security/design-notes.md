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
