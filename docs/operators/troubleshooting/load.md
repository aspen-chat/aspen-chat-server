# Load

## `rateLimited`

**Means:** too many requests to one endpoint. `Retry-After` says for how long.

**What to do:** if many people share one address (an office, a school, a load test):

1. Check `[rate_limits] trusted_proxies` first. Behind a proxy that is not listed, everyone
   counts as the proxy.
2. To lift the per-address limits for a network for a while, the listener's
   `[connections] max_per_ip` and `max_per_network` among them:

   ```
   aspen-chat-server limits suspend --scope networks --network <cidr> --reason …
   ```

## `uploadQuotaExceeded`

**Means:** someone has uploaded the deployment setting `upload-quota-gib` (25 GiB by default)
within the last 24 hours. `detail` and `Retry-After` say when there is room again.

**What to do:** if those uploads are legitimate, raise or clear the limit, in the dashboard or
with:

```
aspen-chat-server settings set --upload-quota-gib N
```

`0` means no limit.

## `serverBusy`

**Means:** one of these:

| Cause | How to tell |
| --- | --- |
| More sign-ins than `[auth] password_hashing_threads` can check within `password_hashing_wait_seconds`. | It comes with sign-ins. |
| Every database connection stayed taken for `database_pool_wait_seconds`. | The server logs "no database connection came free". |
| On sign-in, password reset, registration, and invite endpoints: Valkey cannot be reached to count their rate limits, or is full and refusing writes. | The server logs "rate limits unavailable". |

**What to do:** brief bursts are expected (after an outage, everyone signs in again). If it
lasts:

- **Sign-ins:** add API servers or CPU.
- **Database connections:** the dashboard's Server fleet tab charts, under **Database latency**,
  how long statements took and how long work waited for a connection. Waits rising while
  statements stay as they were mean connections are held, not that the database is slow: check
  NATS first, since every write holds its connection until NATS acknowledges its event, then
  raise `database_pool_size` within what PostgreSQL's `max_connections` allows, or add API
  servers. Statements rising too mean the database itself is slow: look at PostgreSQL's load,
  its disk, and the network to it.
- **Valkey:** bring it back, or raise its `maxmemory` when `INFO memory` shows it full. Those
  endpoints are refused until it answers (`[rate_limits] fail_closed`).

## `internal`

**Means:** something failed on the server.

**What to do:** the log has an `ERROR` line for each, with the cause. Most are the database,
NATS, Valkey, or storage being unreachable.
