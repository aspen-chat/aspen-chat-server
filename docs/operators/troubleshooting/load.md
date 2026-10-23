# Load

## `rateLimited`

**Means:** too many requests to one endpoint. `Retry-After` says for how long.

**What to do:** if many people share one address (an office, a school, a load test):

1. Check `[rate_limits] trusted_proxies` first. Behind a proxy that is not listed, everyone counts
   as the proxy.
2. Signed-in people count only toward their own limits, and people not yet signed in each toward
   their device's, so a shared address rarely holds anyone back. A tool that makes many accounts or
   sign-ins from one device is held to one device's limits.
3. To lift the per-address and per-device limits for a network for a while, the listener's
   `[connections] max_per_ip` and `max_per_network` among them, and let it go without device
   credentials:

   ```
   aspen-chat-server limits suspend --scope networks --network <cidr> --reason …
   ```

## `deviceCredentialRequired`

**Means:** a request without a session presented no device credential (the `Aspen-Device` header),
or one that is not this deployment's or has expired. Aspen's apps earn one on their own and send the
request again, so people do not see this.

**What to do:**

- **A tool or script calling the API:** have it earn a credential
  (`POST /api/v1/auth/device-challenges`, solve the puzzle, then
  `POST /api/v1/auth/device-credentials`) and send it on every request it makes without a session.
  `scripts/device_credential.py` does this.
- **A health check or load balancer:** point it at `GET /api/v1/health` or `/health/ready`, which
  ask for none.
- **An endpoint called by something that cannot solve the puzzle** (another kind of server, say):
  add it to `[rate_limits] device_exempt`. That list replaces the built-in one, so copy the built-in
  entries from `rate_limits.toml` too.

## `deviceChallengeInvalid`

**Means:** a solution to a device credential's puzzle was refused: its challenge expired (five
minutes), was already used, is not this deployment's, or the nonce does not solve it. `detail` says
which.

**What to do:** nothing for Aspen's apps, which ask again. If people's apps keep failing on slow
devices, lower `[rate_limits] device_puzzle_bits`.

## `uploadQuotaExceeded`

**Means:** someone has uploaded the deployment setting `upload-quota-gib` (25 GiB by default) within
the last 24 hours. `detail` and `Retry-After` say when there is room again.

**What to do:** if those uploads are legitimate, raise or clear the limit, in the dashboard or with:

```
aspen-chat-server settings set --upload-quota-gib N
```

`0` means no limit.

## `serverBusy`

**Means:** one of these:

| Cause                                                                                                                                                | How to tell                                         |
| ---------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| More sign-ins than `[auth] password_hashing_threads` can check within `password_hashing_wait_seconds`.                                               | It comes with sign-ins.                             |
| Every database connection stayed taken for `database_pool_wait_seconds`.                                                                             | The server logs "no database connection came free". |
| On sign-in, password reset, registration, and invite endpoints: Valkey cannot be reached to count their rate limits, or is full and refusing writes. | The server logs "rate limits unavailable".          |

**What to do:** brief bursts are expected (after an outage, everyone signs in again). If it lasts:

- **Sign-ins:** add API servers or CPU.
- **Database connections:** the dashboard's Server fleet tab charts, under **Database latency**, how
  long statements took and how long work waited for a connection. Waits rising while statements stay
  as they were mean connections are held, not that the database is slow: check NATS first, since
  every write holds its connection until NATS acknowledges its event, then raise
  `database_pool_size` within what PostgreSQL's `max_connections` allows, or add API servers.
  Statements rising too mean the database itself is slow: look at PostgreSQL's load, its disk, and
  the network to it.
- **Valkey:** bring it back, or raise its `maxmemory` when `INFO memory` shows it full. Those
  endpoints are refused until it answers (`[rate_limits] fail_closed`).

## `internal`

**Means:** something failed on the server.

**What to do:** the log has an `ERROR` line for each, with the cause. Most are the database, NATS,
Valkey, or storage being unreachable.
