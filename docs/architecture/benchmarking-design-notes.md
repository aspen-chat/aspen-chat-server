# Benchmarking: design notes

Why [Benchmarking](benchmarking.md) works as it does.

## Rate limit suspension

- The suspension always ends by itself. It ends at its `until` and never later than each server's own `max_suspension_seconds` after it began, so a forgotten suspension, or one written with a far `until`, cannot leave a deployment open.
- Suspending is a command, never an HTTP endpoint. It runs with the deployment's credentials, so no account, however privileged, can lift the limits that protect a deployment from its own users.

## Purge

- Only cycle-closing nullable references are cleared. Any other is left to the order of deletion, since clearing it could break a check on its table.
- The foreign key graph is read from `pg_constraint` at purge time. A table added with a foreign key to anything a benchmark user can make is then covered without changes to the purge.

## Metrics

- Non-standard methods are named `OTHER`. The request series stay bounded however many made-up methods arrive.
- The bench reports heap growth beside resident growth. Allocated rising tells a leak; resident well above allocated is memory jemalloc keeps. Reporting both tells the two apart.
