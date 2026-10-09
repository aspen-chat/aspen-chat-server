# Federation: design notes

Why federation works as it does. The how is in the [Federation pages](index.md).

## Identity and keys

- **The private key is kept in the database.** So every API server signs with the same one. The cost is that the database's backups are as sensitive as the key. See [Identity and keys](identity-and-keys.md#the-key).
- **Federation needs HTTPS.** Other deployments are reached only at `https://{domain}`, so a deployment at an `http` address takes no part.
- **`rotate-key --compromised` makes no handover.** A key that may be in someone else's hands vouches for nothing, so every deployment that pinned it must accept the new key by hand.

## Gates and lists

- **Lists keep their entries whatever a gate reads.** Switching a gate from an allow list to a block list never turns the allowed into the blocked. See [Gates and lists](gates-and-lists.md#lists).
- **Block lists match parent hosts on any port; allow lists match exactly.** Blocking `evil.org` must also block `a.evil.org` and `evil.org:8443`, while an allow list admits only what it names.
- **Changes to gates and lists sign people out within seconds.** The standing check would otherwise do it only at its next pass. The `shutOut` job is saved with the change.
- **The `shutOut` job decides per home, from the gates as they then stand, not per user.** A hundred users at a time.
- **The standing pass is a recurring job that passes only while a gate is open.** So opening a gate starts it without a restart.

### Every CORS origin is allowed

- **Every deployment allows every CORS origin.** Browsers call other deployments' APIs directly. Every request carries a bearer token rather than a cookie, so no origin gains anything by it. See [Gates and lists](gates-and-lists.md#cors).

## The directory

### An unannounced key change suspends

- **A key not reached through handovers is refused and held as `offered_key`.** A key that changes unannounced may mean someone else answers at its domain, so an administrator accepts it only after confirming the change with the other deployment's administrators. See [The directory](directory.md#pinning-keys).
- **While suspended, even statements that verify against the pinned key are refused.** The pinned key may be one that leaked (`rotate-key --compromised` at the other end). This includes `accountDeleted`.
- **`follow_handovers` is bounded by `MAX_DOCUMENT_KEYS` and tries the `kid` first.** So a document cannot make it verify more than a few hundred signatures.
- **Documents are refreshed at each standing pass, beside the check rather than before it.** So a replaced key stops verifying here within about an interval, planned or not.

### Strangers

- **A deployment not yet known is recorded on first contact.** So whatever contacts strangers must check `admits` first.
- **Statements from unknown or unverifying senders are checked against the sender's own document before anything is recorded.** So a forged statement naming a stranger leaves no row and pins nothing.
- **Unused deployments are forgotten after thirty days.** Not one waiting on a key acceptance, and not one with a note.
- **A deployment on a block list cannot be forgotten.** Forgetting it would otherwise admit it wherever a gate blocks only those listed.

### Outbound calls

- **`PublicResolver` filters addresses as it connects.** So a name cannot pass a check and then resolve elsewhere.
- **`Domain::parse` refuses anything a URL would read as an address.** So no domain passes by being an address.
- **`[federation.development]` is refused unless `public_url` is local.** It adds trusted roots and allows private addresses, for `scripts/dev_federation.py`.

### Contacting statement senders

- **A statement can name any domain as its issuer, and so make this server call it.** So a sender that cannot be reached is refused with one detail however it failed (`deploymentUnreachable` with the `statementSenderUnreachable` message), with the specific failure going to the log only. An administrator contacting a deployment sees the specific cause instead.
- **One sender is contacted for statements at most every fifteen seconds, or a minute after a failure.** A statement in between is checked against whatever key that contact left pinned.

## Signing in abroad

- **The receiver reads only the issuer before verifying, and contacts it only if a gate might admit it.** See [Signing in abroad](signing-in-abroad.md#verifying-it).
- **An assertion that calls a bot a person, or a person a bot, is refused.** The gate checked is the one for the kind the assertion claims.
- **Assertion ids are remembered in Valkey until they expire.** So each is used once.
- **First arrivals per home per day are capped.** So a home that mints accounts cannot fill this deployment with them.
- **A later name this deployment would not allow leaves the held name.** So a rule this deployment tightened never signs anyone out, and never stands in the way of retiring or revoking them.
- **Avatars are copied, and a replaced copy is deleted once unused.** So a home cannot fill the host's storage by changing its avatar.
- **`/federation/icons/{icon}` sends `nosniff` and a sandboxing CSP.** It answers on the web client's origin.

## DMs and notices

- **A deployment hosts a DM only when starting it with one of its own users.** So everyone in it has an account on the host, where its rules (a shared community, blocks) apply. See [DMs across deployments](dms-and-notices.md).
- **Every statement is verified in one place, `received::receive`.**
- **Clients believe `homeId` and `homeDomain` only from the viewer's own home.** Any other deployment could name anyone as its user's home.

## Standing

- **One server runs a pass: the one that claims the recurring `confirmStanding` job.** See [Jobs](../jobs/index.md).
- **A pass is bounded: at most `MAX_DUE_PER_PASS` (5000) users and four minutes (`CONFIRM_BUDGET`).** Those never confirmed go first, then those confirmed longest ago. Chunks not asked within the budget stay due for the next pass rather than counting against their home.
- **No pooled connection is held while waiting on a home.**
- **A failing home is backed off, but the grace still applies.** Its users lose their sessions at `standing_grace_seconds` however long the back-off.
- **Requests name at most 128 users.** So a request and its answer fit within the 16 KiB a statement may be. See [Standing](standing.md#limits).
- **A home answers only for users who signed in at the asker, and `refused` for any other id.** So an answer reveals nothing of accounts the asker was never given.

### `accountDeleted` is taken from any pinned home

- **`accountDeleted` is accepted from any home whose key is pinned, even one no gate admits.** It only takes away. It verifies against the pinned key alone and contacts no one. Every other notice is taken only from a deployment a gate admits. See [Standing](standing.md#account-deletion).

## Protocol evolution

- **Unknown enum values are treated as the most cautious known value.** An unknown `Gate` admits no one; a key of an unknown `KeyAlgorithm` is never used. See [Protocol versions](protocol-versions.md).
- **`Software` never decides anything.** It is for people to read.
- **Aspen's capability list is empty.** Aspen is not yet used outside its developers' machines, so every feature is the baseline of version 1.
