# The server

## The server will not start

It says why on standard error. The usual causes:

- a setting it cannot read;
- no built web client in `[web_client] dir`;
- a rate limit naming an endpoint that does not exist;
- a `public_url` whose host is not the domain this deployment is already known by, or an `http`
  one once it has a domain;
- a service it cannot reach.

## Something that should follow a decision does not happen

For example, a banned person's messages stay, or mail is not sent. It is a job that has not run.

`aspen-chat-server jobs list`, or the dashboard's Jobs tab, shows what is running, what waits,
and what was given up, with its error.

- Jobs wait when no server runs them (`[jobs] run` off everywhere; see
  [`[jobs]`](../configuration/jobs.md#jobs)) or every place is taken.
- A job given up after its attempts is kept. `jobs retry <id>` runs it again once its cause is
  fixed.
- The log has a line for each failed attempt, naming the job and its kind.

## A setting changed but a server did not follow

Every API server watches NATS for changes to the deployment settings (see
[Deployment settings](../configuration/deployment-settings.md#deployment-settings)), and reads them from the database as they commit.

A server that cannot reach NATS logs `following the deployment's settings failed`, and tries
again every five seconds. It reads the settings afresh when it reconnects.

## `aspen-migrate up` stops at "usernames that differ only by case"

Usernames are unique regardless of case. The migration that makes them so names the accounts
that clash.

1. Ask all but one of each group to change their username, under their profile.
2. Run `aspen-migrate up` again.

## Everyone is signed out after an upgrade

It should not happen: sessions are in the database. Check that the new servers point at the same
`database_url`.
