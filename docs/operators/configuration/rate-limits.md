# Rate limits

Every endpoint is rate limited. The built-in limits, with what each is for, are in
`server/app/src/rate_limits.toml`, which also explains the format. What you write here is laid
over them: a limit you give replaces the built-in one whole.

## `[rate_limits]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | |
| `trusted_proxies` | `[]` | Reverse proxies, as addresses or networks (`"10.0.0.0/8"`), whose `X-Forwarded-For` names the client. **Set it when the server is behind a proxy**, or every client counts as the proxy. See [Trusted proxies](#trusted-proxies). |
| `ipv6_prefix` | `64` | IPv6 clients are counted by their network of this many bits. |
| `max_suspension_seconds` | `86400` | The longest this server honours a suspension of the limits (`aspen-chat-server limits suspend`), counted from when it began. |
| `fail_closed` | sign-in, password reset, registration, and invite endpoints | Endpoints refused with `serverBusy` while Valkey cannot be reached to count their limits. See [When Valkey is down](#when-valkey-is-down). |

### Trusted proxies

The client is the right-most `X-Forwarded-For` entry that is not itself a listed proxy.

- Entries are read with or without a port: `192.0.2.1:5678`, `[2001:db8::1]:80`.
- An entry that is not an address (`unknown`) ends the reading there. The client then counts as
  the listed proxy that passed it on.

### When Valkey is down

- Endpoints in `fail_closed` are refused with `serverBusy`.
- Every other endpoint is let through.
- The per-username sign-in limits always fail closed.
- A `fail_closed` list given here replaces the built-in one (in `rate_limits.toml`) whole.

## Sign-in limits by username

Password sign-in (`POST /auth/login`) has two limits by username:

| Limit | What it counts |
| --- | --- |
| `username` | Every attempt at a name from one network: an IPv4 /24, an IPv6 /48, or the coarser `ipv6_prefix`. |
| `username_failures` | Only wrong passwords for the name, from every network together. |

While `username_failures` is spent, password sign-in for that name is refused (`rateLimited`)
until it refills. The owner can still sign in with a passkey or from another device, or reset the
password by email.

## Registration

Registration (`POST /users`) is limited per address and for everyone together: 600 accounts at
once, then twenty a second (72000 an hour). That passes a launch's rush. A deployment expecting
more at once raises it:

```toml
[rate_limits.endpoints."POST /users"]
global = { requests = 6000, per_seconds = 60, burst = 3000 }
```

## Limits for one endpoint

Limits for one endpoint go under its method and path:

```toml
[rate_limits.endpoints."POST /channels/{channel}/messages"]
user = { requests = 10, per_seconds = 10, burst = 5 }
```

A limit that names an endpoint, or a path parameter, that does not exist stops the server at
startup, with a message saying which.
