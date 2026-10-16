# Health

Two endpoints say whether an API server is up, for what stands before it or watches it: load
balancers, reverse proxies, and deployment tools.

## Where it lives

| Part | Code |
| --- | --- |
| The handlers, and a private worker's listener | `api::health` |
| Asking the services, and keeping the answer | `app::health` (`readiness`, `Readiness`, `LastChecked`) |
| Their rate limits | `GET /health` and `GET /health/ready` in `server/app/src/rate_limits.toml` |
| The problem code | `ProblemCode::NotReady` |

## The endpoints

| Endpoint | Answers | Asks |
| --- | --- | --- |
| `GET /health` | `204` whenever the server is taking requests. | Nothing. |
| `GET /health/ready` | `204` when the server reaches its database, NATS, and Valkey. Otherwise `503` `notReady`, whose `detail` names those it does not reach. | The database for `SELECT 1` and Valkey for a `PING`, each given two seconds. NATS is not asked: the client's own account of its connection is read. |

Both are under `/api/v1`, in the OpenAPI document, and need no session.

## Which to ask

- **What routes requests to a server asks `GET /health`.** Every API server shares one
  database, NATS, and Valkey. A check of those would fail on every server at once, and a load
  balancer would then send people its own error instead of the server's, which says what is
  wrong and when to try again.
- **What starts or replaces a server asks `GET /health/ready`**, before it stops the one being
  replaced. So does anyone finding out what is wrong with a server.

A server listens only once it has connected to everything, so both answer from the first
moment either can. Readiness matters afterwards, when a service goes away.

## Cost

Anyone may ask either, and neither is rate limited: what asks does so from one address, many
times a minute, and a limit reached would read as the server being down. Rate limits are also
counted in Valkey, which `GET /health/ready` exists to report on.

So neither may cost more for being asked more:

- `GET /health` does no work.
- `GET /health/ready` keeps its answer for a second (`FRESH`). Requests that arrive while the
  services are being asked wait for that answer rather than asking again, so the database sees
  one statement a second however many ask.

## What it shows, and to whom

Anyone can learn that a server is up, and which of three services it cannot reach. Nothing
about the deployment's people or contents, and no address, credential, or error text: those
are in the server's log, which says once when readiness changes (`this server is not ready`,
with the services, and `this server is ready again`).

A private worker (`--private-worker`) takes no requests, and answers both on a listener of its
own instead, `[health] listen_addr` (`api::health::serve_for_private_worker`), loopback by
default, where a container's own health check asks. The images hold `curl` for asking it.
