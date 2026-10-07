# Federation

Federation lets the people of your deployment use other Aspen deployments with the account they
already have, and other deployments' people use yours. Someone's **home** is the deployment
where they made their account; any other deployment they use is **foreign** to them. Signing in
at home is all they ever do: their home vouches for them to the others.

Nothing crosses until you say so. Every gate is closed by default.

## Before you start

- **A domain that will not change.** The host of an `https` [`public_url`](configuration.md#the-deployments-address)
  is this deployment's name among deployments, such as `chat.example.org` (with `:port` if it is
  not served on 443). Other deployments remember the key they find there, so a deployment that
  changes its domain is a stranger to all of them. The first server to start with it records it
  in the database, and a server started with another domain, or with an `http` address, refuses
  to start.
- **HTTPS with a certificate from a public authority**, such as Let's Encrypt, at
  `https://<domain>`. Other deployments reach yours only there, follow no redirects, refuse
  self-signed certificates, and give up after ten seconds.
- **`/.well-known/aspen` reachable.** It is the document other deployments read: your domain,
  your key, and your gates, served by the API servers like everything at your address. Check it
  with
  `curl https://chat.example.org/.well-known/aspen`.

The first API server to start with a domain makes this deployment's key and keeps it in the
database, so every API server signs with the same one. Back the database up accordingly: see
[Backups](backups.md).

## Gates

Each direction has a gate, for people and separately for bots:

- **Emigration**: your accounts using other deployments.
- **Immigration**: other deployments' accounts using yours.

A gate is `closed` (no one), `open` (everyone), `allowList` (only deployments on its allow
list), or `blockList` (everyone but those on its block list). The gates are deployment
settings, kept in the database: administrators with Manage federation set them in the
dashboard's Federation tab, and the terminal sets them too. Either way every API server follows
at once, without a restart. The usual policies, for people (bots' gates are `--bots-…`):

| You want | `aspen-chat-server settings set` |
| --- | --- |
| No federation | leave both closed |
| Your people may visit others; no visitors | `--users-emigration open` |
| Visitors welcome; your people stay | `--users-immigration open` |
| Both, with anyone | `--users-emigration open --users-immigration open` |
| Both, only with deployments you choose | `--users-emigration allowList --users-immigration allowList` |

A gate opens only once the deployment has a domain, an `https` `public_url`. `--users-shared-list true` makes both
directions read one list instead of a list each, when you think of "the deployments we federate
with" as one set. `--users-immigration-invite-required true` asks a visitor arriving for the
first time for a registration invite, as `registration-invite-required` does of new accounts.
`aspen-chat-server settings show` shows them all.

Closing an immigration gate, or narrowing it, signs out at once the visitors from deployments it
no longer admits, and so does putting a deployment on a block list, taking it off an allow list,
or forgetting it. Closing an emigration gate stops your people signing in elsewhere; the
deployments they are visiting sign them out when they next ask about them (`[federation]
standing_interval_seconds`).

Lists keep their entries whichever gate reads them, so switching a gate from an allow list to a
block list never turns the allowed into the blocked.

A deployment on a block list is blocked with every name under it and on every port: blocking
`evil.org` blocks `chat.evil.org` and `evil.org:8443` too, and `federation list` says when a
deployment is blocked that way. An allow list admits exactly the deployments on it, with the
port each is listed with, and nothing under them.

## The directory

Every deployment this one knows is in the dashboard's Federation tab, with its key's
fingerprint, the lists it is on, and whether it is admitted each way. Administrators with Manage
federation add deployments, put them on lists, and contact them there; the terminal does the
same:

```
aspen-chat-server federation status
aspen-chat-server federation list
aspen-chat-server federation add friends.example.net --note "The book club"
aspen-chat-server federation list-add friends.example.net usersEmigrationAllow
```

A deployment is also recorded the first time it is in contact, as when one of its people signs
in here. One recorded that way that nobody uses (no one of yours uses it, none of its people are
here, it is on no list, and has no note) is forgotten thirty days after it was last contacted,
unless it is waiting for you to accept a new key; give it a note to keep it.

`aspen-chat-server federation remove <domain>` (or Forget in the dashboard) forgets a deployment,
its key and the lists it is on. A deployment on a block list cannot be forgotten, since that
would take it off the list and let it in: take it off its block lists first if you mean to.

### Keys

The first time this deployment reads another's document, it remembers that deployment's key.
From then on:

- The same key: nothing to do.
- A new key the old one handed over to: followed on its own.
- **A new key nothing vouches for: refused.** The deployment is suspended until an
  administrator accepts the new key: everything from it is refused, even what its old key
  signs, since the old key may be the one that leaked, and its people signed in here are signed
  out at once. Ask its administrators, by some other way than Aspen, whether they replaced their
  key, and compare fingerprints before accepting it, in the dashboard or with
  `aspen-chat-server federation accept-key <domain> --fingerprint SHA256:…`. Its people then
  sign in again. A key that changes unannounced can mean someone else is answering at that
  domain.

While any gate is open, this deployment reads again, about every `[federation]
standing_interval_seconds`, the document of each deployment it federates with and that is in
use (one you added, noted, or listed, one whose people are here, or one your people use), so it
notices a replaced key within about that long even when nothing else contacts that deployment.

To replace your own key:

- `aspen-chat-server federation rotate-key --planned` has the old key sign a handover to the
  new one. Every deployment follows it on its own, as long as it last saw a key among your
  sixteen newest from the last ninety days.
- `aspen-chat-server federation rotate-key --compromised` when the old key may be in someone
  else's hands. It vouches for nothing, so every deployment that knew you refuses the new key
  until its administrators accept it: tell them, and give them the new fingerprint
  (`federation status` prints it). Each notices the change at its next check (about every
  `standing_interval_seconds`, an hour by default) or sooner, and from then on refuses
  everything signed as you, old key or new, and signs your people out, until it accepts the new
  key. Until a deployment notices, whoever holds the old key can sign as you there, so tell
  their administrators at once: one who runs `aspen-chat-server federation contact
  <your domain>` notices straight away.

## What people see

Someone whose home lets them emigrate adds another server from the "Create or join a community"
dialog, or opens an invite link naming it (`/invite/<code>?at=<domain>`). Their home signs a
two-minute statement of who they are for that deployment, which signs them in there. Their
profile stays their home's; their status is theirs to set anywhere.

Visitors appear in lists as `name@domain`. Moderators with Ban users can ban a visitor from your
deployment in the dashboard's user directory, as they can your own users: their sessions end
and they cannot sign in here again until the ban ends or is lifted. Banning one of your own users
also tells every deployment they visit, at its next standing check, that they are no longer in
good standing there.

About every `standing_interval_seconds` this deployment asks each visitor's home whether they
are still in good standing. A visitor whose account was deleted, who left, or whose home closed
its gate to you is signed out; so are the visitors of a home unreached for
`standing_grace_seconds`. Homes are asked sixteen at a time, each given twenty seconds, and one
that keeps failing is asked less often, up to once an interval, so slow homes hold up no one
else.

## Protocol versions

Deployments say which versions of the federation protocol they speak, and two with none in
common refuse each other with an error naming both ranges. Every Aspen release keeps speaking
the versions of the last thirty-six months, so keeping up with releases keeps you federating.
`spec/federation.md` is the contract.

## Trying it on one machine

`scripts/dev_federation.py up` runs two deployments, `alpha.localhost` and `beta.localhost`,
with a development certificate authority, and `check` exercises federation between them.
