# Gates and lists

Policy is gates. They are deployment settings (`FederationPolicy`, read from each server's copy of the settings; see [Deployment settings](../administration/deployment-settings.md)).

## Where it lives

| Part | Code |
| --- | --- |
| Gates | `FederationPolicy`, `admits` |
| Lists | `FederationList`, `directory::lists_of`, `directory::set_listed` |
| Host matching | `Domain::blocked_by` |
| Signing out on change | `standing::shut_out_step`, run by the `shutOut` job |

## Gates

There are four gates: `emigration` and `immigration`, for users and, separately, for bots.

| Gate | Governs |
| --- | --- |
| `emigration` | This deployment's accounts using others |
| `immigration` | Others' accounts using this one |

Each gate is one of:

| Value | Meaning |
| --- | --- |
| `closed` | The default. No one |
| `open` | Everyone |
| `allowList` | Only deployments on its allow list |
| `blockList` | Everyone but deployments on its block list |

The modes operators speak of are the users' gates: none, emigration, immigration, and full, each open or with a list.

A gate's first arrivals may also need a registration invite (`immigration_invite_required`, for each kind of account); see [Signing in abroad](signing-in-abroad.md#the-foreign-user).

## Lists

A gate with a list reads one of twelve lists (`FederationList`). Each is named by who, which way, and allow or block: `usersEmigrationAllow` … `botsSharedBlock`.

- With `shared_list`, both directions read the `Shared` list, and must then be the same kind.
- Every list keeps its entries whether or not a gate reads it. Switching a gate from an allow list to a block list never turns the allowed into the blocked.

## Deciding admission

`admits` decides from the gates and the lists that decide a deployment (`directory::lists_of`):

- the lists it is on;
- every block list a deployment is on whose host is its host or a parent of it, on any port (`Domain::blocked_by`).

So blocking `evil.org` blocks `a.evil.org` and `evil.org:8443` too. An allow list admits only the deployments it names, exactly.

Each directory record carries the result as `admission`, beside the lists it is on itself.

## Changing gates and lists

- Holders of Manage federation change the gates from the dashboard (`PATCH /admin/federation`), and the terminal from `settings set`.
- Every server follows at once.
- A gate opens only once the deployment has a domain.

These changes, from the dashboard or the terminal, sign out the users of other deployments the gates no longer admit:

- a change to an immigration gate or its shared list;
- a deployment put on a list or taken off one (`directory::set_listed`);
- a deployment forgotten (`directory::remove`).

Each change saves with it a job (`shutOut`, urgent; see [Jobs](../jobs/index.md)). The job (`standing::shut_out_step`):

- signs them out within seconds, a hundred at a time;
- decides per home, from the gates as they then stand, rather than per user.

The standing check would otherwise do so at its next pass (see [Standing](standing.md)).

## The standing pass job

The standing pass is a recurring job (`confirmStanding`; see [Jobs](../jobs/index.md)). It does a pass only while a gate is open, so opening one starts it without a restart.

## CORS

Browsers call other deployments' APIs directly, so every deployment allows every CORS origin. **Why:** see [design notes](design-notes.md#every-cors-origin-is-allowed).
