# The Aspen federation protocol

What one Aspen deployment says to another, and what a client of one deployment says to another
deployment's API, and how both change over time without breaking deployments that run other
versions or forks of Aspen. `docs/architecture/federation.md` describes how this implementation does it;
this document is the contract any implementation keeps.

The payloads deployments exchange are described by `federation_schema.json` (root type
`FederationProtocol`), which `aspen-chat-server --gen-openapi-schema` writes beside
`openapi.yaml` and `event_schema.json`. The REST API and the event stream, which clients of other
deployments use too, are described by those two.

## What deployments exchange

- **The document**, served unauthenticated at `https://{domain}/.well-known/aspen`: the
  deployment's domain, its keys (the current one first, then those it replaced within the
  handover window, each with the handover that vouches for it), its gates, its `protocol`, and
  its `software`. A reader follows handovers through no more than the first sixteen keys a
  document lists, so a deployment lists at most that many.
- **Signed statements**: compact JWS (RFC 7515) with EdDSA over Ed25519 (RFC 8037), at most
  16 KiB (16384 bytes) each, since a reader refuses a longer one. The header's `typ` names the
  kind, and a verifier checks it, so no statement passes for another:
  - `aspen-assertion+jwt`: a home deployment says who one of its users is, how they signed in,
    and their profile, to one other deployment (`aud`), for at most five minutes, once (`jti`).
  - `aspen-key-handover+jwt`: a deployment's outgoing key vouches for its next.
  - `aspen-notice+jwt`: a deployment tells one user's home something about them, POSTed as
    `{"notice": "…"}` to `https://{home}/api/v1/federation/notices`, which answers `202` for
    any notice it verifies, of a kind it knows or not. The notice's `kind` says what it is
    about; the kinds so far:
    - `dmJoined` (`channel`, `by`): the user is in a DM on the sender, started with them or
      with them added. The home passes it on to the user's devices only while the user still
      uses the sender.
    - `accountDeleted`: sent by a home to every deployment its user used; the account is gone,
      and each retires its user. Since it only takes away, a deployment takes it from any home
      whose key it has pinned, whatever its gates now say of that home.
  - `aspen-standing-request+jwt`: a deployment asks one home, POSTing `{"request": "…"}` to
    `https://{home}/api/v1/federation/standing`, about that home's users signed in to it (by
    their ids at home, at most 128, so that the request and its answer each fit in a
    statement), about hourly. A home answers for at most the first 128 a request names, and an
    asker takes a user the answer leaves out as no news of them.
  - `aspen-standing+jwt`: the home's answer, `{"standing": "…"}`, saying of each user `good`
    (the account exists and may still use the asker), `gone` (the account was deleted), or
    `refused` (it may no longer use the asker). A home answers only for users who signed in at
    the asker, and says `refused` of anyone else, whether or not they have an account, so an
    answer tells the asker nothing about accounts it was never given. An asker ends the sessions of a user it hears
    `refused`, or a standing it does not know, of; retires one it hears `gone` of; and ends the
    sessions of users whose home it has not reached for a day.

A deployment hosts a DM only while starting it with at least one of its own users in it; it
may go on after they leave. Everyone in it therefore has an account on the host, and the host's
own rules (a community shared with whoever started it or added them, blocks) apply there.

## Versions and capabilities

A deployment says which protocol it speaks in its document and in `GET /api/v1/auth/methods`:

```json
"protocol": { "version": 1, "minimum": 1, "capabilities": [] },
"software": { "name": "aspen", "version": "0.1.0" }
```

- `version` is the newest protocol version it speaks and `minimum` the oldest. Two deployments,
  or a client and a deployment, speak the newest version both know; when their ranges do not
  meet, they do not federate, and each says so rather than failing half way.
- `capabilities` names what it can do beyond its version's baseline. Aspen's own capabilities
  have bare names (`dms.federated`); a capability a fork adds is named by a domain the fork
  controls, reversed (`org.example.reactions3`), so no name Aspen adds later can collide with
  it. Before using a feature that is a capability, a deployment or client checks the other
  side has it, and offers something else, or says it cannot, when it does not.
- `software` is for people to read, as the Administration Dashboard shows it. Nothing may be
  decided by it: a fork's version numbers mean nothing to anyone else.

## How the protocol changes

1. **Within a version, only by addition.** A field, value, event, statement, endpoint, or
   capability may be added. Nothing is removed, renamed, or given a new meaning, and nothing that
   was optional becomes required.
2. **Readers ignore what they do not know.** An unknown field is skipped; an unknown enum value
   reads as an explicit unknown and is treated as the most cautious known value (an unknown gate
   admits no one; a key of an unknown algorithm is never used); an unknown event, statement, or
   notice type is ignored. A request another deployment's client may send is not refused for a
   field the server does not know; only requests that just this deployment's own clients send
   (its Administration Dashboard, its security settings) are strict, to catch mistakes.
3. **A change that cannot be an addition is a new version.** It gets its own paths (`/api/v2`),
   `typ`s, and document fields, served beside the old ones, and the `minimum` rises only once no
   supported deployment needs the old version.
4. **Support window.** Every deployment speaks every protocol version released within the last
   thirty-six months, so a deployment updated at least that often federates with every other.
5. **A feature added later is a capability** until the version that makes it baseline, and
   readers check for it as above.

Aspen is not yet in use outside its developers' machines, so every feature so far is the
baseline of version 1, and the capability list is empty. The rules above bind once it is.

## Keeping to it

- `spec/fixtures/federation/` holds payloads of every released version, and of an imagined newer
  deployment that uses everything the rules allow it to add; the server's tests read them all.
  A release adds its own.
- `scripts/dev_federation.py up --alpha-bin … --beta-bin …` runs the two local deployments from
  different builds, which is how two versions are checked against each other.
