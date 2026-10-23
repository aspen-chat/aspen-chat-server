# Step 4: Starting the API server

## With a certificate

```
aspen-chat-server --cert fullchain.pem --key privkey.pem
```

| Flag            | What it does                                                                                                               |
| --------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `--port`        | The port to listen on. 443 by default.                                                                                     |
| `--listen-addr` | The address to listen on. May be given more than once. Every interface over IPv4 and IPv6 by default (`0.0.0.0` and `::`). |

- **An address you give must bind**, or the server refuses to start.
- Given none, a machine without IPv6 listens over IPv4 alone, and says `not listening on [::]:443`
  in its log.
- An IPv6 address takes IPv6 only. To listen over one family, give its address alone:
  `--listen-addr 0.0.0.0`.
- Open the port over both families; an IPv6 firewall is usually a separate one.

The server does not notice a renewed certificate. Restart it after renewing.

Without `--cert` and `--key`, and without `--no-https` (below), it refuses to start.

## Behind a reverse proxy

When a reverse proxy terminates TLS, run the server without TLS on a private address:

```
aspen-chat-server --no-https --listen-addr 127.0.0.1 --port 8080
```

Then tell it which proxies to believe about client addresses:

```toml
[rate_limits]
trusted_proxies = ["127.0.0.1"]
```

**Without this, the rate limits count every client as the proxy.**

Pass every path through unchanged, those outside `/api/v1` included. Two are read by others than
browsers, which give up at a redirect or at anything but the server's own answer:

- `/.well-known/aspen`, which other deployments read (see
  [Federation](../federation/before-you-start.md#well-knownaspen-reachable)).
- `/.well-known/assetlinks.json`, which Google reads before Android phones let the app use the
  deployment's passkeys (see
  [Android apps and passkeys](../configuration/accounts.md#android-apps-and-passkeys)). It must
  answer `200` with `Content-Type: application/json`, over `https` at port 443, with no redirect. Do
  not serve a file of your own there: the server's names the apps it accepts.

## Health checks

Point whatever stands before the server at one of these. Neither needs a session, and neither is
rate limited.

| Ask                        | Answers                                                                                                                                                   | For                                                                                      |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `GET /api/v1/health`       | `204` while the server is running.                                                                                                                        | A load balancer's or reverse proxy's health check.                                       |
| `GET /api/v1/health/ready` | `204` when it also reaches its database, NATS, and Valkey; otherwise `503` [`notReady`](../troubleshooting/server.md#notready), naming those it does not. | Waiting for a new server before stopping the one it replaces, and finding what is wrong. |

A [private worker](#private-workers) answers both at the same paths on a listener of its own.

**Use the first for a load balancer.** Every server shares the same services, so with the second a
database outage would take every server out of the load balancer at once, and people would see its
error page instead of the server's own message.

## Outbound requests

The server fetches some things from addresses others choose: link previews, plugins' calls, other
deployments, and the push services phones name.

- It fetches them directly, refusing private addresses.
- It ignores `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`.
- Let the API servers reach the internet on port 443 (and 80, for link previews) without a proxy.

## Running several

Run as many API servers as you need. They share everything through the services, and any of them can
serve any request.

## Private workers

Every API server also does a share of the deployment's background work:

- sending mail and making digests,
- applying the voice servers' reports,
- confirming visitors with their home deployments,
- running plugins' observers and timers,
- waking phones.

To give that work machines of its own, start a server as a private worker:

```
aspen-chat-server --private-worker
```

A private worker:

- takes no requests and holds no event streams;
- answers the [health checks](#health-checks) on a listener of their own, `[health] listen_addr`
  (`127.0.0.1:9467` by default; see [`[health]`](../configuration/metrics.md#health));
- needs no web client;
- takes none of the listening or TLS flags;
- reads the same `aspen.toml` and needs the same services;
- serves its metrics like any other server, when they are on.

To keep the SMTP credentials on a machine that takes no traffic, turn `[email] send` on for a
private worker and off for the other servers.

---

Previous: [Step 3: Configuring and migrating](3-configuring.md) · Next:
[Step 5: Serving the web client](5-web-client.md)
