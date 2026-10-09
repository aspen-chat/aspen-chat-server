# Timers

Code: `timer` (`timer::set`, `timer::cancel`, `timer::fire_step`, `timer::wake`).

Each plugin's timer is a job (`firePluginTimer`, normal; see [Jobs](../jobs/index.md)), due when the timer is.

## Setting

Timers are set and cancelled by key (`timer::set`, `timer::cancel`).

- The job is keyed by the plugin and the timer's key: `plugin/key`. A plugin's id, a reversed domain, cannot confuse it.
- Setting a key again replaces its job whole under a new id. A run of the timer it replaces finishes without deleting it.
- The job's payload holds what it hands the plugin, and its owner as storage records one (`ownerKind`, `owner`).

| Call | Owner | Scope |
| --- | --- | --- |
| `set-timer` | The deployment. | — |
| `set-timer-in` | As storage's for the scope. | Recorded (`scopeKind`, `scope`), checked as storage's by `host::Call::scope`. |

- At most `timer::MAX_TIMERS_PER_OWNER` (1000) per owner, counted when a new key is set, through `job_plugin_timer_owner`.
- Two set at once may each find room, so an owner may hold a few more.

## Firing

The job (`timer::fire_step`) hands the timer to `observe::deliver` as `timer-fired` when its plugin:

- is on;
- holds `timers`;
- observes `timer.fire`.

| Case | What happens |
| --- | --- |
| The call fails | Tried again, three times in all, then kept as failed for the operator. |
| The plugin is off | The timer waits ten minutes at a time. Turning the plugin on makes those that fell due meanwhile due at once (`timer::wake`). |
| The plugin is removed | The timer is done with. The purge takes the rest. |

## Deleting

`storage::forget` deletes the timers of a scope with its values, through `job_plugin_timer_scope`. Deleting a channel, a community, or an account deletes the timers set there.
