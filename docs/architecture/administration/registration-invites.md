# Registration invites and dual invites

A registration invite lets someone make an account. A dual invite is one that also joins the new account to a community.

## Where it lives

| Part | Code |
| --- | --- |
| Invites and redemption | `app::registration_invite`, table `registration_invite` |
| Dashboard endpoints | `/admin/registration-invites` |
| Public lookup | `GET /registration-invites/{code}` |
| Terminal | `aspen-chat-server invites create\|list\|revoke` |
| Staleness window | `app::invite::STALE_AFTER_DAYS` |

## Registering with an invite

The deployment setting `registration_invite_required` ([Deployment settings](deployment-settings.md)) decides whether one is needed.

| Case | Result |
| --- | --- |
| Required, no `inviteCode` on `POST /users` | Refused with `registrationInviteRequired` |
| Required, an invite that is not usable | Refused with `registrationInviteInvalid` |
| Optional, a code that works | Recorded |
| Optional, a code that does not work | Ignored |

`GET /auth/methods` says which applies, as `registrationInviteRequired`.

## Usability

A registration invite may be used `max_uses` times, until `expires_at` if set, and not once revoked.

- It is redeemed under a row lock in the transaction that makes the account, so concurrent registrations cannot overdraw it.
- The account records it (`user.registered_with`).
- When the last use is taken, `used_up_at` is set.

## Listing and revoking

Administrators make and revoke invites at `/admin/registration-invites`.

- The list shows every usable invite.
- For a week after an invite stops working (`app::invite::STALE_AFTER_DAYS`), it is listed too.
- An invite stops at the first of being revoked, being used up, and expiring.
- The terminal's `invites list` shows them all.

Community invites follow the same week:

- A community's invite list (`GET /communities/{community}/invites`) shows invites that have expired for a week.
- That list is newest first, a page of at most `LIST_PAGE` (100) at a time after the invite `before`. It is read through the index `invite_listed`, or `invite_listed_by_creator` for a member who sees only their own.
- A revoked community invite is gone at once.

## The first account of an invite-only deployment

1. Make the deployment invite-only before anyone has an account, with `aspen-chat-server settings set`.
2. Make an invite from the terminal: `aspen-chat-server invites create [--uses N] [--expires 7d] [--note …]`.
3. The owner registers with it, then grants themselves administration (`admin grant`; see [Deployment roles](deployment-roles.md#the-top-role-and-the-terminal)).

## Dual invites

A dual invite is a registration invite that names a community invite (`registration_invite.community_invite`).

### Making one

Holders of Manage registration invites make one with `community` on `POST /admin/registration-invites`, from the dashboard or a community's invite dialog.

- The community invite is made by them (`app::invite::insert`), so it needs Create invites there like any invite. A non-member, or a moderator without it, is refused.
- It expires with the registration invite.
- It is listed and announced among the community's invites to those who manage them.
- The dashboard's record names the community and whether its invite still works (`community`, read for a page in one query).

### Joining

- The account it makes joins that community in the same transaction (`app::registration_invite::join_invited`, through `app::community::add_member`, which announces the membership and checks community bans).
- A foreign user's first arrival with it does the same (`app::federation::abroad`; see [Signing in abroad](../federation/signing-in-abroad.md)).
- Joining passes over a community invite that no longer works, or a deleted community, rather than refusing the account.

### The public lookup

`GET /registration-invites/{code}` is unauthenticated and limited per address against guessing.

- It shows a usable invite to someone about to register.
- It includes the community invite and, sideloaded, the community.
- So the page can say which community the account joins, and send someone already signed in to join it instead.

### Revoking

| Revoked | Effect |
| --- | --- |
| The dual invite, from the dashboard or `invites revoke` | Its community invite is revoked in the same transaction. `invites revoke` connects to NATS to announce it |
| The community invite, from the community | The registration invite stays, and accounts it makes join nothing |

### Access

- What decides it is the registration invite's usability and the community invite's, both checked as the account is made.
- An account already made is an ordinary member, removed or banned like any.
- `scripts/check_permissions.py` checks making, joining, and both revocations.
