# The directory of other deployments

Every other deployment known is a row of `federated_deployment`. This page covers how rows are made, how keys are pinned, how a deployment is suspended, and how calls to other deployments are made.

## Where it lives

| Part | Code |
| --- | --- |
| Rows, lists, pruning | `app::federation::directory` (`IN_USE_SQL`, `set_listed`, `remove`, `prune_unused`) |
| Contacting | `contact` (`fetch_document`, `record_contact`, `follow_handovers`, `accept_key`, `contact::contact_verifying`) |
| Refreshing documents | `standing::refresh_documents` |
| Outbound calls | `app::federation::fetch`, `app::outbound::PublicResolver`, `is_public_address` |
| Development settings | `[federation.development]`, `AspenConfig::check_federation_development` |
| Dashboard | `/admin/federation`, `/admin/federation/deployments[/{domain}[/contact\|/key\|/lists/{list}]]` |
| Terminal | `aspen-chat-server federation …` |

## The row

A `federated_deployment` row holds:

- how it came to be (`origin`): added by an administrator or from the terminal, or recorded when first contacted;
- a note;
- its pinned key, and any `offered_key`;
- when it was first and last contacted;
- its protocol (see [Protocol versions](protocol-versions.md)).

A deployment not yet known is recorded as first contacted when it is contacted. **So whatever contacts strangers must check `admits` first.**

## Pinning keys

Contacting one (`contact`) runs `fetch_document`, then `record_contact`. It reads the deployment's document and compares the key it presents.

| Key presented | Result |
| --- | --- |
| None pinned yet | The key is pinned |
| The pinned key | `confirmed` |
| A new key reached from the pinned one through the document's handovers | Pinned in its place (`handedOver`) |
| Any other | Refused (`keyChanged`) and kept as `offered_key` |

`follow_handovers` walks back from the current key through at most the document's first `MAX_DOCUMENT_KEYS` keys. It tries first the key each handover's `kid` names. So a document cannot make it verify more than a few hundred signatures.

### Accepting an offered key

An administrator accepts exactly the offered key (`accept_key`), after confirming the change with its administrators. From the terminal, `federation accept-key` takes the offered key's fingerprint.

### Suspension

Until the offered key is accepted, the deployment is suspended:

- `received::receive` refuses everything it signs, even what verifies against the pinned key, `accountDeleted` notices included (`statementKeyPending`).
- `record_contact` saves a job signing out its users here (`standing::shut_out` with its home; see [Jobs](../jobs/index.md)).
- Each standing pass does so again.

**Why:** the pinned key may be one that leaked; see [design notes](design-notes.md#an-unannounced-key-change-suspends).

## Refreshing documents

Every standing pass reads again the document of each deployment that:

- is in use (`directory::IN_USE_SQL`): added by an administrator, noted, on a list, the home of a user here, or used by one of this deployment's users;
- a gate admits;
- has its key pinned;
- was last contacted more than `standing_interval_seconds` ago.

This is `standing::refresh_documents`. The longest contacted go first, all within two minutes, beside the standing check rather than before it.

So a key a deployment replaced stops verifying here within about an interval. A planned replacement's handover is followed. An unplanned one suspends the deployment.

## Verifying a statement from a stranger

A statement whose sender has no key pinned, or that does not verify against the pinned one:

1. has its sender contacted (`contact::contact_verifying`);
2. is checked against the key the sender's document presents;
3. only then is anything recorded.

So a forged statement naming a stranger leaves no row and pins nothing.

## Forgetting deployments

- A deployment recorded on first contact and not in use is forgotten, with its pin, once it has gone uncontacted for thirty days (`directory::prune_unused`, at each standing pass).
- Not if it waits on an administrator to accept a key it offered.
- A note keeps one.
- Forgetting a deployment (`directory::remove`) drops its pin and its list entries.
- One on a block list is refused (`409`) until it is taken off its block lists. **Why:** forgetting it would otherwise admit it wherever a gate blocks only those listed.

## Calls to other deployments

Calls go through `app::federation::fetch`. They:

- are HTTPS only;
- follow no redirect;
- time out after ten seconds;
- read at most 64 KiB;
- connect only to public addresses.

### Public addresses only

- `app::outbound::PublicResolver` filters as it connects, so a name cannot pass a check and then resolve elsewhere.
- No domain passes by being an address, since `Domain::parse` refuses whatever a URL would read as one.
- Link previews refuse the same addresses through `is_public_address`.
- `is_public_address` counts as public only what lies outside every special-purpose network IANA lists.
- An IPv6 address that carries an IPv4 one (mapped, compatible, NAT64, or 6to4) is judged by the IPv4 address.

### Development

`[federation.development]` adds trusted roots and allows private addresses, for `scripts/dev_federation.py`. A server refuses to start with it unless `public_url`'s host is `localhost`, a name under it, or a loopback address (`AspenConfig::check_federation_development`).

### Failures

| Who contacted | Result |
| --- | --- |
| An administrator contacting a deployment | `502` `deploymentUnreachable`, whose detail says which failure |
| A sender contacted for a statement (`received::contact_sender`: signing in abroad, notices, standing requests) | Refused with `deploymentUnreachable` and one detail however it failed (the `statementSenderUnreachable` message). The specific failure goes to the log |

One sender is contacted for statements at most every fifteen seconds, or a minute after it could not be reached. A statement in between is checked against whatever key that contact left pinned. **Why:** a statement can name any domain as its issuer; see [design notes](design-notes.md#contacting-statement-senders).

## Managing the directory

Holders of Manage federation manage it from the dashboard. The terminal (`aspen-chat-server federation …`) does the same, and alone replaces this deployment's key (see [Identity and keys](identity-and-keys.md#replacing-the-key)).
