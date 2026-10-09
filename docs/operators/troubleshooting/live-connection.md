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
| 4429 | Too many live connections to this API server. See [4429](#4429). |

### 4429

The account, or the address it connects from, already holds as many live connections to this
API server as `[limits] max_event_streams_per_user` or `max_event_streams_per_address` allow. The
app keeps trying, and connects once another closes.

Raise the cap, or list the proxy, when:

- many people are behind one address, or
- a reverse proxy is missing from `[rate_limits] trusted_proxies`, so every client seems to come
  from it.

## Connections drop every minute or so

A proxy between clients and the server is closing idle WebSockets. The server pings every thirty
seconds, so any idle timeout must be longer than that.

## Connections never open

Check that the proxy passes `Upgrade` and `Connection` through for `/api/v1/events`.
