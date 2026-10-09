# Settling and resyncs

Events are published before their transaction commits. Settling notes what each piece of work published, rechecks calls it may affect, and announces a resync when work fails after publishing.

## Noting

Every request runs inside `app::events::noting`, through `app::events::settle_after`.

- `settle_after_request` is a layer around every API route.
- It handles each request in a task of its own, which runs to its end even when its client goes away. So abandoning a request between its publishing and its commit rolls nothing back.
- Work dropped part way all the same, by a panic, is settled as having failed.
- Background tasks and operator commands that change access also run inside `noting`.

`noting` records, for each event:

- the communities and users it was published to;
- the transaction it was published in (`pg_current_xact_id`), when inside one and not in a savepoint within it;
- the calls it may change access to (`app::events::rechecks_of`).

An access-changing event published outside `noting` is logged as an error.

## Settling

`app::events::settle` runs once the work is done.

1. It rechecks the noted calls (see [Voice](../voice/index.md)).
2. If the work failed, it reads each publishing transaction's status (`pg_xact_status`). A transaction it cannot answer for counts as not committed.
3. It announces a resync for what was published outside a transaction, or in one that did not commit.

A failure after the publishing transaction committed announces nothing. What it published is true.

## Resyncs

| Published to | Resync | What dispatchers do | What clients do |
| --- | --- | --- | --- |
| A community | `communityResync` for each | Drop the community's model. Connections reading it resume with it loaded afresh. | Read the community again. |
| A user's own subject (a DM, a membership, a setting) | `userResync` to each | Drop the user's connections, which register afresh with their communities and roles read from the database. | Read everything they hold again, as when a resume is refused. |
