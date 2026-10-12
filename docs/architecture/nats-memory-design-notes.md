# NATS memory: design notes

Why [NATS memory](nats-memory.md) works as it does.

## The sizes are deployment settings

- There is one stream of each kind for the whole deployment, so its size is one answer every server must agree on. A size in each server's `aspen.toml` could differ between servers, and each would resize the stream to its own as it started.
- A change takes effect without restarting anything, and is checked against NATS as it is made.
- `[nats] monitor_url` stays in `aspen.toml`: it is an address on the deployment's own network, which a server needs before it can fit the streams at all.

## Automatic by default

- A fixed default large enough for a big deployment (8 GiB of events reserves 8 GiB) is refused by any NATS host with less memory than that, before the deployment has a single user. A fixed default small enough for every host is too small for a big deployment.
- Sizing to what NATS has lets `max_memory_store` be the one number an operator sets. Aspen assumes it may reserve all of it that no other stream has.
- The fallback where the limit cannot be read (1 GiB and 128 MiB) fits a NATS host with 2 GiB of memory under NATS's default limit of three quarters of it.

## The monitoring port

- A NATS server tells an account its own limit only when the account has one. The server's `max_memory_store` is said on the monitoring port and to NATS's system account.
- The monitoring port needs no credentials of its own and is plain HTTP, so it is the smaller thing to ask of an operator. It has no authentication, which is why the operator guide keeps it on the deployment's own network.
- In a NATS cluster each server has a limit of its own, and the port named is one server's. A stream NATS placed on another server is sized to a limit that is not its own, and NATS's refusal is what catches one too large.

## The voice reports' share

- Reports wait in their stream only while no API server applies them, and a voice server's next snapshot repairs any that are let go. They need far less than events, so they take an eighth, and never more than 1 GiB.

## Refusing in the transaction

- `fit` runs inside the settings change's transaction, so a size NATS refuses is never saved, and the person who chose it reads why.
- Fitting last, just before the doorbell, leaves little that can fail after the streams change. What can is put right by `fitStreams`.

## A recurring job, not a watch

- NATS announces neither its own restart nor a stream's loss to an account in a way a server can rely on, so the streams are checked on a schedule.
- A job runs on one server at a time, where a task on every server would have each read NATS and the monitoring port every period for the same answer.
- A failed pass is logged and answered `Done`, since a recurring job's backoff (up to an hour) is longer than the streams should be missing.
