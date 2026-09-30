# Federation

Federation lets the people of your deployment use other Aspen deployments with the account they
already have, and other deployments' people use yours. Someone's **home** is the deployment
where they made their account; any other deployment they use is **foreign** to them. Signing in
at home is all they ever do: their home vouches for them to the others.

Nothing crosses until you say so. Every gate is closed by default.

## Before you start

- **A domain that will not change.** `[federation] domain` is this deployment's name among
  deployments, such as `chat.example.org` (with `:port` if it is not served on 443). Other
  deployments remember the key they find there, so a deployment that changes its domain is a
  stranger to all of them.
- **HTTPS with a certificate from a public authority**, such as Let's Encrypt, at
  `https://<domain>`. Other deployments reach yours only there, follow no redirects, refuse
  self-signed certificates, and give up after ten seconds.
- **`/.well-known/aspen` routed to the API server.** It is the document other deployments read:
  your domain, your key, and your gates. Check it with
  `curl https://chat.example.org/.well-known/aspen`.

The first API server to start with a domain makes this deployment's key and keeps it in the
database, so every API server signs with the same one. Back the database up accordingly: see
[Backups](backups.md).

## Gates

Each direction has a gate, for people and separately for bots:

- **Emigration**: your accounts using other deployments.
- **Immigration**: other deployments' accounts using yours.

A gate is `closed` (no one), `open` (everyone), `allowList` (only deployments on its allow
list), or `blockList` (everyone but those on its block list). The usual policies are:

| You want | `[federation.users]` |
| --- | --- |
| No federation | leave both closed |
| Your people may visit others; no visitors | `emigration = "open"` |
| Visitors welcome; your people stay | `immigration = "open"` |
| Both, with anyone | `emigration = "open"`, `immigration = "open"` |
| Both, only with deployments you choose | `emigration = "allowList"`, `immigration = "allowList"` |

```toml
[federation]
domain = "chat.example.org"

[federation.users]
emigration = "open"
immigration = "blockList"
```

`shared_list = true` makes both directions read one list instead of a list each, when you think
of "the deployments we federate with" as one set. `immigration_invite_required = true` asks a
visitor arriving for the first time for a registration invite, as `[registration]
invite_required` does of new accounts.

Lists keep their entries whichever gate reads them, so switching a gate from an allow list to a
block list never turns the allowed into the blocked.

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
in here.

### Keys

The first time this deployment reads another's document, it remembers that deployment's key.
From then on:

- The same key: nothing to do.
- A new key the old one handed over to: followed on its own.
- **A new key nothing vouches for: refused.** Everything from that deployment is refused until
  an administrator accepts the new key. Ask its administrators, by some other way than Aspen,
  whether they replaced their key, and compare fingerprints before accepting it, in the
  dashboard or with `aspen-chat-server federation accept-key <domain> --fingerprint SHA256:…`.
  A key that changes unannounced can mean someone else is answering at that domain.

To replace your own key:

- `aspen-chat-server federation rotate-key --planned` has the old key sign a handover to the
  new one. Every deployment follows it on its own.
- `aspen-chat-server federation rotate-key --compromised` when the old key may be in someone
  else's hands. It vouches for nothing, so every deployment that knew you refuses the new key
  until its administrators accept it: tell them, and give them the new fingerprint
  (`federation status` prints it).

## What people see

Someone whose home lets them emigrate adds another server from the "Create or join a community"
dialog, or opens an invite link naming it (`/invite/<code>?at=<domain>`). Their home signs a
two-minute statement of who they are for that deployment, which signs them in there. Their
profile stays their home's; their status is theirs to set anywhere.

Visitors appear in lists as `name@domain`. Moderators with Moderate any community can ban a
visitor from your deployment in the dashboard's user directory: their sessions end and they
cannot sign in here again until the ban is lifted. A deployment's own users are never banned
this way; moderate them as usual.

About every `standing_interval_seconds` this deployment asks each visitor's home whether they
are still in good standing. A visitor whose account was deleted, who left, or whose home closed
its gate to you is signed out; so are the visitors of a home unreached for
`standing_grace_seconds`.

## Protocol versions

Deployments say which versions of the federation protocol they speak, and two with none in
common refuse each other with an error naming both ranges. Every Aspen release keeps speaking
the versions of the last thirty-six months, so keeping up with releases keeps you federating.
`spec/federation.md` is the contract.

## Trying it on one machine

`scripts/dev_federation.py up` runs two deployments, `alpha.localhost` and `beta.localhost`,
with a development certificate authority, and `check` exercises federation between them.
