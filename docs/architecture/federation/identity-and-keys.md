# Identity and keys

A deployment is its domain and an Ed25519 key pair. It publishes both, with its gates, in a document other deployments read.

## Where it lives

| Part | Code |
| --- | --- |
| Domains | `app::federation::Domain` (`parse`, `blocked_by`) |
| Keys and rotation | `app::federation::keys`, table `federation_key` |
| The document | `DeploymentDocument`, `GET /.well-known/aspen` |
| Signing and verifying | `aws-lc-rs` |

## The domain

`app::federation::Domain` is:

- a DNS name of two or more labels;
- lowercase;
- with `:port` when not 443;
- never an IP address.

It is the host of `public_url` when that is `https`. A deployment at an `http` address has no domain and takes no part.

Federation needs HTTPS. Other deployments are reached only at `https://{domain}`. The domain is pinned in the deployment settings as `federation_domain`; see [Deployment settings](../administration/deployment-settings.md#federation_domain).

## The key

- The key pair is Ed25519, in `federation_key`.
- The current key is the one not retired.
- It is made by the first server to start with a domain, with `aws-lc-rs`, which also signs and verifies.
- **The private key is kept in the database, which makes the database's backups as sensitive as the key.** Every API server signs with the same one.

## The document

`GET /.well-known/aspen` serves the `DeploymentDocument`.

- It holds the domain, keys, and gates, and the protocol it speaks (see [Protocol versions](protocol-versions.md)).
- Keys: the current key first, then those it replaced within `HANDOVER_WINDOW_DAYS` (ninety), sixteen at most (`MAX_DOCUMENT_KEYS`).
- It is outside `/api/v1` and unauthenticated.
- It is cacheable for five minutes.
- It answers `404` without a domain.

## Replacing the key

Only the terminal replaces this deployment's key (`app::federation::keys`).

| Command | What it does |
| --- | --- |
| `federation rotate-key --planned` | The outgoing key signs a handover: a JWS naming the new key. Deployments that pinned any key of the chain follow it on their own |
| `federation rotate-key --compromised` | Makes no handover. Every deployment that pinned the old key refuses the new one until its administrators accept it |

How other deployments follow a handover, or suspend this one, is in [The directory](directory.md#pinning-keys).

## Fingerprints

Fingerprints are `SHA256:` followed by the key's SHA-256 digest in base64, as SSH prints them. `federation accept-key` takes one.
