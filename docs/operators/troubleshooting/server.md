# The server

## The server will not start

It says why on standard error. The usual causes:

- a setting it cannot read;
- no built web client in `[web_client] dir`;
- a rate limit naming an endpoint that does not exist;
- a `public_url` whose host is not the domain this deployment is already known by, or an `http` one
  once it has a domain;
- a service it cannot reach.

## `notReady`

**Means:** `GET /api/v1/health/ready` found that this server cannot reach a service it needs. The
server logs `this server is not ready` once, naming which of PostgreSQL, NATS, and Valkey, and
`this server is ready again` when they are back. The answer itself names them only on a private
worker's health listener, since anyone may ask a server that takes requests.

**What to do:** for each service named, check that it is running, that this server's machine can
reach it, and that `aspen.toml` gives its address and credentials
([Services](../configuration/services.md)). The server keeps trying by itself and needs no restart.

## NATS does not have the memory for the streams

The server stops as it starts, or a change to a stream's size is refused, saying NATS does not have
the memory. NATS sets a stream's whole size aside when the stream is made, out of `max_memory_store`
(three quarters of the machine's memory when its configuration does not say).

Do one of these:

- Choose sizes that fit:
  `aspen-chat-server settings set --event-stream-mib 256 --voice-report-stream-mib 32`. This works
  while no server runs.
- Give NATS more: raise `max_memory_store` in its configuration and restart it.
- Let Aspen size the streams to what NATS has: name NATS's monitoring port as `monitor_url` under
  `[nats]`, and leave the sizes `auto`.

See [NATS memory](../configuration/services.md#nats-memory).

## Something that should follow a decision does not happen

For example, a banned person's messages stay, or mail is not sent. It is a job that has not run.

`aspen-chat-server jobs list`, or the dashboard's Jobs tab, shows what is running, what waits, and
what was given up, with its error.

- Jobs wait when no server runs them (`[jobs] run` off everywhere; see
  [`[jobs]`](../configuration/jobs.md#jobs)) or every place is taken.
- A job given up after its attempts is kept. `jobs retry <id>` runs it again once its cause is
  fixed.
- The log has a line for each failed attempt, naming the job and its kind.

## A setting changed but a server did not follow

Every API server watches NATS for changes to the deployment settings (see
[Deployment settings](../configuration/deployment-settings.md#deployment-settings)), and reads them
from the database as they commit.

A server that cannot reach NATS logs `following the deployment's settings failed`, and tries again
every five seconds. It reads the settings afresh when it reconnects.

## `aspen-migrate up` stops at "usernames that differ only by case"

Usernames are unique regardless of case. The migration that makes them so names the accounts that
clash.

1. Ask all but one of each group to change their username, under their profile.
2. Run `aspen-migrate up` again.

## Everyone is signed out after an upgrade

It should not happen: sessions are in the database. Check that the new servers point at the same
`database_url`.
