# Sign-in and security: design notes

Why sign-in and security work as they do. The how is in the [main pages](index.md).

## Usernames

See [Usernames](usernames.md).

- **No `@` in a username.** It sets a tag apart.
- **No whitespace, control, or format characters.** By these, two names that look alike could
  differ. Format characters (zero-width spaces and joiners, direction marks and isolates, the soft
  hyphen, tags) are invisible.
- **Names must be NFC.** So one name has one spelling.

## Per-username rate limits

See [Password sign-in](password-sign-in.md#per-username-rate-limits).

- **`username` counts per client network, not globally.** So one network's guesses cannot lock the
  owner out from anywhere else.
- **`username_failures` is peeked before the check and counted only after a wrong password.** So the
  owner's right passwords never spend it.
- **Passkeys, device links, and a reset by email stay open while `username_failures` is spent.** The
  owner keeps a way in that a guesser cannot close.
- **An unknown name counts like a known one.** So the limits tell nothing of which names exist. For
  the same reason, a sign-in naming no account or a bot is checked against
  `login::UNMATCHABLE_PASSWORD_HASH`, taking as long as a wrong password.

## Password hashing limits

See [Password sign-in](password-sign-in.md#password-hashing-limits).

- **Argon2 work runs on a bounded number of threads, and a request that waits too long is refused
  with `serverBusy`.** A burst of sign-ins then costs a bounded amount of memory and leaves the rest
  of the server its CPU. It sheds what it cannot serve soon rather than slowing everyone down.
- **No database connection is held while waiting for or doing hashing work.** Otherwise a flood of
  sign-ins could hold the pool idle and refuse every other request (`database_pool_wait_seconds`).

## Tokens

See [Sessions and signing out](sessions.md#tokens).

- **Tokens are kept only as their SHA-256 digests.** A copy of the database signs no one in.
- **A refresh token is not rotated when used.** It stands for its sign-in until it expires or the
  sign-in ends.
- **A refresh token of a deleted or banned account answers `invalidToken`, as an ended sign-in's
  does.** So the app signs out, and signing in again tells of a ban.

## Attempt limits

See [Second factors](second-factors.md#attempt-limits).

- **A daily window as well as a quarter-hour one, independent of how the quarter hours fall.** So
  the codes cannot be guessed slowly either.
- **Each attempt is counted before it is checked, in one script.** So attempts sent all at once are
  checked no more than ten times.
- **The count is per user.** That lets someone who knows the password keep it reached and lock the
  owner's authenticator codes and password re-verification for a day.
- **Recovery codes and passkeys are checked outside the limits, and a reset by email does not use
  them.** A recovery code is 50 random bits, one of ten, beyond guessing at any pace the endpoints
  allow. So the owner always has a way in that the lock does not close.

## Passkeys

See [Passkeys](passkeys.md).

- **A discoverable credential is required at registration.** So passkey sign-in needs no username.
- **The apps hand ceremonies to a page on the deployment.** A browser offers a passkey only to pages
  under `rp_id`, which the desktop and mobile apps are not.
- **The return address is a loopback port or the `aspen:` scheme, never a web page.** The web client
  runs ceremonies in its own page, and a web page that forwarded its query would hand the return
  code on.
- **The ceremony id travels in the URL fragment.** So it reaches no server log.
- **A credential is reported unknown only when the user handle's account does not hold it.** The
  client has the browser forget what is reported unknown, so another account's passkey, offered for
  the wrong ticket, must never be. Reporting it tells a caller only whether a credential id it
  already holds belongs to an account, and credential ids are random.
- **The server states the user handle rather than leaving the client to derive it.** It is the
  account id's bytes because registration made it so (`start_passkey_registration`), which a test
  holds; the client need not know that.
- **`signalAllAcceptedCredentials` is sent with the list just read, after every change.** A manager
  hides what is missing from it, so it is only ever sent whole. A passkey added elsewhere meanwhile
  may be hidden until the panel next reads, which shows it again.
- **The hand-off page sends only `signalUnknownCredential`.** Anyone holding its link can open it,
  so it is given nothing about the account, and it does not take the account's list from what a
  ceremony's options happen to hold.
- **Autofill starts follow what the person does.** Each start is counted toward the sign-in limits
  and stores state in Valkey for five minutes. Restarting on a timer would spend a device's budget,
  and everyone's, on pages nobody is looking at.
- **Autofill ends before its ceremony expires rather than restarting by itself.** A pick after
  expiry would fail, and starting another would be a start nobody asked for.

### The return code

- **The claim needs a one-time return code as well as the verifier.** The page's link is a real link
  on the deployment's own origin, so whoever starts a ceremony can send it to someone else. The
  verifier alone would let them claim what that person's passkey approved: a session in their
  account for a `signIn`, or a verification of a stolen session for a `reauthenticate`.
- Every return address the server accepts leads back to the device whose browser ran the page (its
  own loopback or its own app). So the code never reaches someone who sent the link elsewhere, and a
  ceremony is claimed only where it was run.
- The page also tells whoever opens it to continue only for a request they made in the app on that
  device.

### Android apps

- **The app runs ceremonies through Credential Manager where the deployment names it, and hands them
  to the browser otherwise.** One app build serves every deployment. A deployment names it in its
  own `assetlinks.json`, which Android checks; the app's manifest names no domain, so nothing in the
  build limits which deployments it reaches.
- **The browser stays the fallback.** Credential Manager may be missing, find no provider, or refuse
  the app while Google has not read the deployment's file; each of those hands the ceremony to the
  system browser rather than failing it.
- **The app asks Credential Manager only for passkeys at hand on the phone.** With none for the
  deployment, Google's sheet would wait to be dismissed, which reads as cancelled. Asked so, it
  fails at once, and the browser, which also reaches security keys and other phones, takes over.
- **Only `get_login_creds`.** App Links' `handle_all_urls` would claim the deployment's web
  addresses for the app, which it does not open.
- **No release certificate is built in yet.** The official app has no release signing key. Debug
  builds are each signed with their builder's own key, so an operator names those they trust.
- **iOS cannot do the same.** An iOS app may use a domain's passkeys only if its entitlements list
  the domain (`webcredentials:`), and they are signed into the build. One build cannot list every
  deployment, so the iOS app keeps the hand-off.

## Device links

See [Device links](device-links.md).

- **A signed-in phone may not scan an offer.** Claiming would replace its sign-in.
- **One scan takes the link (`SET NX`), and the loser gets `deviceLinkUsed`.** This tells someone
  whose screen was photographed.
- **Cancelling uses `SET XX` for scans and approvals in flight.** So a cancelled code stays
  cancelled.
- **Only the receiver can claim, with its verifier.** A giver approving a stranger's request, or a
  stranger holding a photographed code, gets no session.
- **Nothing about the devices is looked up or recorded beyond the name each gives itself.**
- **The new sign-in copies the giver's `method` and `verified_at`.** So it is no stronger than the
  giver's.
- **Giving a sign-in takes a recent verification.** Without it a stolen session could mint a sign-in
  of its own on another device, outliving the stolen one.
- **Starts are limited globally as well as per address.** Each start keeps state in Valkey until it
  expires.
- **Nothing is announced; both sides poll.** A link lives for a few minutes at most and has no
  event.

### User codes

- **Every link a current app starts has a user code, of both kinds.** Whoever would have scanned may
  type instead, so a television, a console, a desktop, or a command line can take either side.
- **A link has a code only when its starter asks (`withCode`).** An app that predates codes shows
  none, and a code nobody shows is only something to guess.
- **RFC 8628's base-20 alphabet, eight letters.** It has no vowels, so no words, and no letter
  mistaken for a digit. Eight letters are about 34.5 bits, short enough to type on a remote.
- **Kept under its digest, reserved with `SET NX`.** As the link's id is kept, and no two live links
  share a code.
- **Five minutes rather than one.** Typing takes longer than scanning: reading eight letters,
  perhaps with a remote, perhaps after the giver confirms it's them with a password and a code. The
  QR code lasts as long, since a photograph of the screen shows the letters too. Links without a
  code keep their minute.
- **Wrong codes are failures, of the address, the account, and everyone.** A code can be guessed
  where an id cannot, so what bounds guessing is how many may prove wrong. The global ceiling is
  what a crowd of addresses cannot widen.
- **Each entry is counted before its code is looked at, and given back if right.** Counted only
  after proving wrong, entries sent at once would all pass the look taken before any had failed, and
  a guesser could send a thousand for each one the ceiling allows.
- **The global ceiling refuses everyone's typed codes once spent.** That lets a guesser stop typed
  codes for a while, which is better than letting them keep guessing. QR codes still work.
- **Every miss answers alike.** An unknown, expired, or cancelled code, and one of the other side's
  kind, are all `deviceLinkCodeUnknown`. Telling them apart would tell a guesser which codes are
  live.
- **A request taken by a typed code waits for its receiver to confirm the giver's account.** A
  signed-in guesser who matched a stranger's code could otherwise sign the stranger's device in to
  the guesser's account, which the stranger would then use as their own. The receiver sees the name
  and picture, and declines an account that is not theirs.
- **The confirmation is a field of the claim, not a step of its own.** The receiver already proves
  itself to the claim with its verifier, so naming the account there adds no endpoint and no state.
- **A QR request asks for no confirmation.** Its id cannot be guessed, so its giver saw the
  receiver's screen, as the receiver meant. Asking would add a tap to the common path and guard
  against nothing new.
- **A typed offer asks for nothing more than a scanned one.** A guesser who matched an offer's code
  becomes a receiver the giver must approve by its name, as a photographed QR code's thief does, and
  the person who meant to type it is told `deviceLinkUsed`.

## Device credentials

See [Device credentials](device-credentials.md).

- **People behind one address are told apart by something they hold, not by where they are.** A
  carrier-grade NAT puts thousands of people behind one IPv4 address, and an event's network
  everyone in the room. Counted by address alone, they would share one person's limits. Counted by
  device, each has their own, and the address keeps only a ceiling high enough for a crowd.
- **A credential costs a puzzle, once.** A credential that cost nothing would let one machine be as
  many devices as it asked for. The puzzle makes every device one machine claims cost it the same
  work a person pays once per deployment. It is not meant to stop a determined attacker, whose
  hardware solves it far faster than a phone, but to make each device cost something.
- **SHA-256 rather than a memory-hard puzzle.** It needs no dependency on either side, and the
  server checks a solution with two hashes, so made-up solutions cost it nothing. A memory-hard
  puzzle would narrow what a GPU gains, but every check of a solution would then cost the server as
  much as one try costs the client.
- **Challenges and credentials are stateless.** Every API server checks them with the shared key and
  asks no one, so presenting a credential adds no lookup to a request. Only a challenge's use is
  remembered, until it expires, so that one solution earns one credential.
- **Networks have a ceiling of their own, everyone a high one.** An address is cheap to change
  within a network one holds (an IPv6 /48 is 65536 of the /64s an address counts as), so the sign-in
  endpoints and redeeming a credential also count by network, at four addresses' worth. Everyone's
  redemptions together are limited only to bound what Valkey keeps of used challenges: a limit low
  enough to matter would let anyone with enough networks spend it, and every new install needs a
  credential before it can sign in. A difficulty that rises under load would raise the cost of
  spending it further; the fixed difficulty and the high ceiling are what there is.
- **Ninety days, renewed past half its life.** A device in use never solves the puzzle again, and
  one hoarded or copied ages out.
- **Signed-in requests count by user alone.** A session already says who is asking, and it is
  something an address's people each hold. Counting their requests against their address too would
  hold everyone behind a NAT to one budget as soon as many were signed in.
- **Except where nobody signs in.** On password sign-in, second factors, registration, token
  refresh, and resets, a session gives nothing the handler uses, so counting by it would only let
  anyone holding a token (their own, or a bot's they made) skip the address's, the device's, and the
  failure limits that hold guessing there, and spray passwords across names with the hashing pool as
  the only limit. On those routes a request counts as its address whatever session it presents; a
  valid session spares it only the device credential, since signed-in apps (this version's and
  others') read `GET /auth/methods` and `GET /deployment` with their session and hold none. Where a
  session is optional, it does count, so each address limit set there needs a user limit beside it,
  which the limiter checks when it compiles the configuration rather than leaving to whoever writes
  it.
- **Session and bot tokens are signed, so a made-up one is refused without the database.** Looking
  up a token takes the database, and signed-in requests are not counted by their address before
  their token is read. A limit on an address's rejected tokens, checked before each lookup, would
  hold back everyone signed in behind that address while one machine there sent made-up tokens. A
  token's signature tells a made-up one apart with no lookup, so nothing needs that limit. Only bot
  tokens from before signing keep it, since only a lookup tells them from made-up ones, and a bot
  cannot refresh to a signed one as an app does.
- **A signed token the database rejected is remembered.** Otherwise one machine could present its
  own ended session over and over, and each time cost a lookup.
- **Failures are counted, not just attempts.** A crowd mostly knows its passwords and its invites,
  and a machine guessing does not. Counting what proves wrong lets an address's attempts be generous
  while its wrong guesses are few.
- **Token refresh and the passkey page are exempt.** A phone's push handler refreshes its session
  without the app running, and cannot solve the puzzle. A refresh token is a secret only its holder
  knows, so a valid one is counted only by the address's ceiling, and unknown ones as failures. The
  passkey page is a page the system browser opens, not an Aspen client. Its calls each name a
  ceremony only its starter knows.
- **The client earns a credential when asked, not ahead of time.** Then a server that asks for none,
  or a stub in a test, costs it nothing, and the first refusal costs one round trip more.
- **A credential survives signing out.** It names the device, not the person, and a person signing
  out and in again would otherwise pay the puzzle each time.

## Identity providers

See [Identity providers](identity-providers.md).

- **Aspen is a relying party only.** Issuing tokens others depend on (an OAuth authorization server,
  "Sign in with Aspen") waits for an outside security audit.
- **Identities are matched by provider and `sub`, never by email address.** An address can be
  reused, changed, or asserted by a provider that does not own it, so matching by it would let one
  person's provider sign in as someone else.
- **The claim needs the client's PKCE verifier and a one-time return code, as a passkey hand-off's
  does.** Whoever starts a flow can send its authorization URL to someone else; the return code
  reaches only the device whose browser ran the flow, so the effect goes only to whoever started it
  there. The web client's return page is this deployment's own, at a path the server fixes, so no
  page a client names can be handed the code.
- **The server keeps its own PKCE verifier and nonce for the provider.** The provider's code is
  exchanged with a verifier the client never sees, so a code obtained elsewhere cannot be spent on a
  flow, and `state` (the flow's id) may pass through the provider's logs.
- **Nothing takes effect until the claim.** A flow whose browser never comes back, or whose return
  reaches the wrong device, does nothing.
- **A provider's sign-in counts as one factor unless it says otherwise.** Its `amr` claim, or the
  administrators' word (`enforces_two_factor`), decides; a provider that says nothing is treated as
  a password, so `require_two_factor` and an account's own second factors still apply.
- **Abroad, a provider's sign-in travels as `password` or `secondFactor`.** Other deployments then
  read it as any other, and the federation protocol names no provider.
- **Accounts made through a provider have no password.** They verify again at the provider, asked to
  sign in afresh (`prompt=login`, `max_age=0`, checked by `auth_time`), and may set a password
  later. Removing the last way in is refused, so no account is left that no one can sign in to.
- **Removing a provider keeps its row and leaves its sign-ins.** The sign-ins name it, and deleting
  a provider's many links is a job. Ending every sign-in it made would be another data-driven job; a
  provider that should end them sends back-channel logouts.
- **Logout tokens are verified here, not by `openidconnect`,** which verifies ID tokens only. Only
  asymmetric signatures from the provider's published keys are taken, a token carrying a `nonce` is
  refused so an ID token cannot pass for one, and each `jti` is used once.
- **Providers on private networks need the operator's word.** Holders of Manage identity providers
  are administrators of the deployment, not of its machines; the terminal's
  `allow_private_addresses` keeps them from pointing the server at what only it can reach.
