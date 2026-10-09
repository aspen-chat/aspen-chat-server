# Step 4: Starting the API server

## With a certificate

```
aspen-chat-server --cert fullchain.pem --key privkey.pem
```

| Flag | What it does |
| --- | --- |
| `--port` | The port to listen on. 443 by default. |
| `--listen-addr` | The address to listen on. Every interface by default. May be given more than once. |

The server does not notice a renewed certificate. Restart it after renewing.

Without `--cert` and `--key`, it makes a self-signed certificate for `localhost`. That only suits
development.

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

## Outbound requests

The server fetches some things from addresses others choose: link previews, plugins' calls,
other deployments, and the push services phones name.

- It fetches them directly, refusing private addresses.
- It ignores `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`.
- Let the API servers reach the internet on port 443 (and 80, for link previews) without a
  proxy.

## Running several

Run as many API servers as you need. They share everything through the services, and any of
them can serve any request.

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

- opens no listening socket, so it takes no requests and holds no event streams;
- needs no web client;
- takes none of the listening or TLS flags;
- reads the same `aspen.toml` and needs the same services;
- serves its metrics like any other server, when they are on.

To keep the SMTP credentials on a machine that takes no traffic, turn `[email] send` on for a
private worker and off for the other servers.

---

Previous: [Step 3: Configuring and migrating](3-configuring.md) · Next:
[Step 5: Serving the web client](5-web-client.md)
