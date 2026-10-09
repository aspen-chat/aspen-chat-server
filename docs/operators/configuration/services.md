# Services

Where the API server finds PostgreSQL, NATS, and Valkey.

## The services

| Setting | Default | |
| --- | --- | --- |
| `database_url` | required | PostgreSQL. See [`database_url`](#database_url). |
| `database_pool_size` | two per logical CPU | The most PostgreSQL connections this server holds at once. See [Sizing the pool](#sizing-the-pool). |
| `database_pool_wait_seconds` | `10` | How long a request or background task waits for a pool connection before it is refused with `serverBusy`. |
| `nats_url` | required | NATS with JetStream, as `host:4222`, or `tls://host:4222` over TLS. The server warns at startup when NATS on another machine is reached without TLS. |
| `nats_auth_token` | | The token NATS was started with, when NATS takes one token from everyone. |
| `[nats_user] user`, `password` | | The API servers' NATS user, when NATS has users. Give exactly one of this and `nats_auth_token`. |
| `[nats.tls]` | | TLS settings for NATS. See [TLS to the services](#tls-to-the-services). Given, TLS is required whatever `nats_url` names. |
| `valkey_url` | required | Valkey, as `redis://host:6379`, or `rediss://host:6380` over TLS. The server refuses to start with a password in a `redis://` address unless the host is this machine. |
| `[valkey.tls]` | | TLS settings for Valkey. See [TLS to the services](#tls-to-the-services). Needs a `rediss://` `valkey_url`. |

### `database_url`

Give it either way:

- `postgres://user:password@host/database`
- libpq's `host=… user=…` pairs

It takes libpq's TLS parameters:

| Parameter | |
| --- | --- |
| `sslmode` | `disable`; `prefer` (the default) and `require`, which encrypt without checking the certificate unless `sslrootcert` names a file; `verify-ca`; `verify-full`. |
| `sslrootcert` | A PEM file of the authorities to trust, or `system`. |
| `sslcert`, `sslkey` | A client certificate and its key. |

`aspen-migrate` and the operator commands read it the same way. See
[Network exposure](../installing/network-exposure.md#postgresql) for how to set PostgreSQL up for
TLS.

### TLS to the services

`[nats.tls]` and `[valkey.tls]` take the same keys:

| Key | |
| --- | --- |
| `ca_file` | PEM authorities to trust besides the system's. |
| `cert_file`, `key_file` | A PEM client certificate and its key. Give both or neither. |

### Sizing the pool

- Every write holds a connection until NATS acknowledges its event.
- So a busy server may run out of connections before PostgreSQL runs out of CPU.
- The database connections metric shows requests waiting.
- Keep the total over every API server below PostgreSQL's `max_connections`.
- When the pool runs dry (NATS is slow to acknowledge, or the server has more work than
  connections), work waits `database_pool_wait_seconds` and is then refused with `serverBusy`.

### NATS users

NATS has users once each voice server has one of its own. Then set `[nats_user] user` and `password`
for the API servers. See [Voice servers](../installing/6-voice-servers.md#give-it-a-nats-user).

## Development credentials are refused

`docker-compose.yaml` and the development scripts publish passwords and keys in Aspen's
repository: `aspen_test`, the storage keys, and `[media.s3]`'s defaults.

A server whose `public_url` is `https` at a host other than `localhost` (or a name under it)
refuses to start with any of them as:

- `database_url`'s password
- `nats_auth_token`
- `[nats_user] password`
- `[media.s3] access_key` or `secret_key`

It says which to change.
