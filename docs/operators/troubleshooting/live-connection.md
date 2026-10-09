# The live connection

The app keeps a WebSocket open to `/api/v1/events`.

## Close codes

When the server closes the connection on purpose, it sends a message saying why, then closes
with one of these codes:

| Close code | Why |
| --- | --- |
| 4400 | The first message was not a well-formed `identify`. An outdated client, or a proxy mangling the connection. |
| 4401 | The session is not valid: expired, signed out elsewhere, or revoked, before the stream opened or while it was open (signing out, a password or second factor change, a deleted account). The app signs in again. |
| 4403 | Two factors are required and the account has none, when the stream opens or when the requirement is turned on while it is open. |
| 4408 | No `identify` within ten seconds. Usually a very slow connection. |
| 4410 | The account was banned from this deployment. The app signs out and says why. |
| 4429 | Too many live connections to this API server, or an account reconnecting too often. See [4429](#4429). |
| 1013 | The API server is busy taking on many connections at once. See [1013](#1013). |

### 4429

One of:

- The account already holds as many live connections to this API server as `[limits]
  max_event_streams_per_user` allows (`tooManyStreams`). The app keeps trying, and connects once
  another closes.
- The address it connects from already holds as many connections that have not yet signed in
  as `max_event_streams_per_address` allows (`tooManyStreamsFromAddress`).
- The account reconnected faster than `GET /events`'s limits per user allow (`rateLimited`). The
  app waits as long as the message says, then connects. Something reconnecting in a loop, such as
  a misbehaving bot, is the usual cause.

Raise `max_event_streams_per_address`, or list the proxy, when:

- many people behind one address connect at once, or
- a reverse proxy is missing from `[rate_limits] trusted_proxies`, so every client seems to come
  from it.

### 1013

So many connections arrived at once that the API server is serving those already connected first
(`serverBusy`): it signs in at most `[limits] max_identifying_event_streams` at a time, and none
while requests wait for a database connection. The app comes back by itself after the ten seconds
the message names, spread over as long again. It is expected for a few minutes after a server
restarts or many people's network comes back. If it lasts, the deployment needs more API
servers, or a larger `database_pool_size` and a matching PostgreSQL `max_connections`.

## Connections drop every minute or so

A proxy between clients and the server is closing idle WebSockets. The server pings every thirty
seconds, so any idle timeout must be longer than that.

## Connections never open

Check that the proxy passes `Upgrade` and `Connection` through for `/api/v1/events`.
