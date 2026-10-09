# Gates

Each direction has a gate, for people and separately for bots:

| Gate | Controls |
| --- | --- |
| Emigration | Your accounts using other deployments. |
| Immigration | Other deployments' accounts using yours. |

A gate is one of:

| Value | Who crosses |
| --- | --- |
| `closed` | No one. |
| `open` | Everyone. |
| `allowList` | Only deployments on its allow list. |
| `blockList` | Everyone but those on its block list. |

A gate opens only once the deployment has a domain: an `https` `public_url`.

## Setting the gates

The gates are deployment settings, kept in the database. Set them either:

- in the dashboard's Federation tab, as an administrator with Manage federation, or
- with `aspen-chat-server settings set` in the terminal.

Either way, every API server follows at once, without a restart. `aspen-chat-server settings
show` shows them all.

## The usual policies

For people (bots' gates are `--bots-…`):

| You want | `aspen-chat-server settings set` |
| --- | --- |
| No federation | Leave both closed. |
| Your people may visit others; no visitors | `--users-emigration open` |
| Visitors welcome; your people stay | `--users-immigration open` |
| Both, with anyone | `--users-emigration open --users-immigration open` |
| Both, only with deployments you choose | `--users-emigration allowList --users-immigration allowList` |

Other options:

| Option | What it does |
| --- | --- |
| `--users-shared-list true` | Both directions read one list instead of a list each. Use it when you think of "the deployments we federate with" as one set. |
| `--users-immigration-invite-required true` | A visitor arriving for the first time needs a registration invite, as `registration-invite-required` does of new accounts. |

## Lists

- Lists keep their entries whichever gate reads them. Switching a gate from an allow list to a
  block list never turns the allowed into the blocked.
- A deployment on a block list is blocked with every name under it, and on every port. Blocking
  `evil.org` blocks `chat.evil.org` and `evil.org:8443` too. `federation list` says when a
  deployment is blocked that way.
- An allow list admits exactly the deployments on it, with the port each is listed with, and
  nothing under them.

To put a deployment on a list, see [The directory](directory.md).

## What closing a gate does

| Change | Effect |
| --- | --- |
| Closing or narrowing an immigration gate | Visitors from deployments it no longer admits are signed out at once. |
| Putting a deployment on a block list, taking it off an allow list, or forgetting it | Same: its visitors are signed out at once. |
| Closing an emigration gate | Your people can no longer sign in elsewhere. The deployments they are visiting sign them out when they next ask about them (`[federation] standing_interval_seconds`). |
