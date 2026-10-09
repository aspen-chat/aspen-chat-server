# The dashboard and fleet health

The Administration Dashboard is the deployment's console. This page covers what it reads: totals, growth, the directories, and the fleet.

## What the dashboard reads

| Endpoint | What it shows | Permission |
| --- | --- | --- |
| `/admin/overview` | The deployment's totals | View dashboard |
| `/admin/growth?range=…` | Growth of users and communities | View dashboard |
| User and community directories | Users and communities, searched, sorted, and paged | View dashboard, or any moderation permission but Message any user (`DeploymentPermissions::DIRECTORIES`) |
| `/admin/moderation-log` | The [moderation log](moderation.md#reading-the-log) | View dashboard |
| `/admin/file-transfers` | The record of files offered in calls ([File transfers](../file-transfers.md)) | Moderate any community |
| `/admin/registration-invites` | [Registration invites](registration-invites.md) | Manage registration invites |
| The fleet | [Fleet health](#fleet-health) | View dashboard |
| `/admin/jobs` | A preview of background jobs ([Jobs](../jobs/index.md)) | View jobs |

Powers too strong for even a privileged web API, such as suspending rate limits, stay terminal commands.

## Totals

`/admin/overview` (`admin::current_totals`) reads the latest row of `deployment_stats`.

- The recurring job `recordStats` ([Jobs](../jobs/index.md)) writes it each hour, keeping one row a day.
- The totals are that row plus what was made and deleted since it was taken.
- Each of those is found through an index of its own, so the overview never counts a whole table.

## Growth

`range` is `threeMonths`, `sixMonths`, `oneYear`, `fiveYears`, or `allTime`.

- Points are users and communities at the end of each day up to half a year, each week up to two years, and each month beyond.
- Each step's point is the last day recorded in `deployment_stats` before the step ends.
- The last step's point is the totals now.
- The migration that made `deployment_stats` filled in every day before it.

## Directories

The user and community directories:

- search by any part of a name through `filter[name]`, or for one or two characters by its start (`admin::name_pattern`), served by trigram indexes either way;
- order by `sort`, a field, `-` prefixed for descending, newest first by default, each order walked through an index (`user_dashboard_name`, `community_dashboard_name`, `community_by_members`, the key for when it was made);
- count a community's members as `community.member_count`, which the recurring job `recountMembers` ([Jobs](../jobs/index.md)) recounts each hour;
- page by `offset`, at most `MAX_OFFSET` (10,000), and `limit`, 15 by default. **Why:** see [design notes](design-notes.md#directories-page-by-offset).

The user directory:

- leaves out the system account, as the totals and the growth chart do, since it is the deployment itself;
- takes `filter[banned]=true`, and gives each standing ban as `ban`;
- marks users of other deployments by their domain;
- is one of the directories every moderation permission opens.

## Fleet health

Fleet health (`app::fleet`) comes from heartbeats.

1. Every `HEARTBEAT_INTERVAL` (ten seconds), each API server writes a heartbeat to the NATS key-value bucket `aspen_fleet`. Private workers do too, and show no streams or requests.
2. The heartbeat holds the server's uptime, open event streams, requests and server errors per minute, resident memory, and database connections.
3. Entries expire after thirty seconds, so a server that stops drops out.

The figures are counted where the matching Prometheus metrics are recorded. The heartbeat is how the dashboard sees every server without reaching their loopback metrics endpoints.

### Voice servers

Voice servers' health is their registry rows: enabled, capacity, and whether their last report is within `offer_silence_seconds`.

### Unregistered voice servers

A load report from an id no registered server has (a voice server given the wrong `id`) matches no row.

1. The API server applying it notes the id in `aspen_fleet`, under `voice.unregistered.` (`fleet::note_unregistered_voice_server`).
2. The note lasts as long as a heartbeat does, so it stays while the voice server keeps reporting every fifteen seconds.
3. The fleet lists these as `unregisteredVoiceServers`.
4. The dashboard shows each with what to do, and shows every registered server's id beside them, to copy.
