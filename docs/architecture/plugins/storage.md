# Storage

Code: `storage`, table `plugin_storage`.

## Keys

Storage is keyed by plugin, scope, and key in `plugin_storage`.

## `storage-swap`

`storage-swap` writes or deletes a value only while it holds what the caller expected (`storage::swap`).

- It compares and writes under the row's lock in one transaction.
- A write that finds a key absent and loses the race to make it is tried again.
- For a swap expecting the key absent, losing that race is refused instead.

## Scopes

A scope must be there:

- a community or channel not deleted;
- a person not deleted and, for a plugin turned on community by community, a member of one where it is on.

It must be where the plugin runs. In a route it must also be where the caller may look:

- a channel they may view;
- a community they belong to;
- themselves.

So a plugin cannot multiply its quota by naming owners that do not exist.

## Owners and quotas

Each scope draws on an owner's share (`storage::Owner::of`):

| Scope | Owner |
| --- | --- |
| A community, its channels, and their threads | The community. |
| A DM or group DM, and its threads | The DM, by its channel. |
| A user | The user. |
| The deployment | The deployment. |

- `plugin_storage_usage` counts each owner's keys and values against `storageQuota`.
- It counts in the write's transaction, and only when a value's length changes.
- One community filling its share leaves every other's room.
- A plugin's total is its shares' sum (`storage::totals`), read for the operator and the dashboard. No write waits on another owner's.

### The total limit

A plugin's total is held to `[plugins] storage_total_gib` (16 GiB; 0 sets no limit).

- Each server keeps each plugin's sum for ten seconds (`storage::total_full`, `Plugins::storage_totals`).
- While the total is at the limit, a write may shrink or delete a value but not grow one.
- Servers writing at once may together pass the limit by what they write in those ten seconds.

## Listing by prefix

Keys compare by their bytes: `plugin_storage.key` is in the `C` collation. So `storage-list` reads a prefix as a range of the key's index, from the prefix to the least string past it (`past_prefix`), however much else the scope holds.

## Forgetting

`forget` deletes a scope's values: a channel's with its threads', a community's with its channels'.

- It is called from `delete_channel`, `delete_community`, and `user::retire`.
- It works by a job saved in their transactions (`forgetPluginScope`; see [Jobs](../jobs/index.md)).
- The job deletes a batch at a time, taking each batch off its owner's count.
- A community's or a user's count goes with them once all is gone.
- `storage::forget` deletes the timers of a scope with its values, through `job_plugin_timer_scope` (see [Timers](timers.md)).
