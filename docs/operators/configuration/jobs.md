# Jobs

Background jobs are work a request decides but does not wait for, such as deleting a banned
person's recent messages, and upkeep that runs on a schedule. Making previews is a job too (see
[Previews](previews.md#how-previews-are-made)).

## How jobs run

- Jobs wait in the database, so none is lost to a restart.
- Every server that runs jobs takes its share.
- A server that stops part way through a job leaves it to another within a minute.
- The dashboard's Jobs tab, and `aspen-chat-server jobs list`, show what they are doing.

## `[jobs]`

| Setting | Default | |
| --- | --- | --- |
| `run` | `true` | Whether this server runs jobs. At least one server of the deployment must. |
| `concurrency` | two per logical CPU | The most jobs this server runs at once that any class of job may take. Each class (urgent, interactive, normal, bulk, maintenance) also keeps one place of its own. |
