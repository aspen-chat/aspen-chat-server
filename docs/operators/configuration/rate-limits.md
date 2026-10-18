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
| `device_exempt` | health checks, the passkey page, token refresh, federation, unsubscribe, the web client's pages, plugins' assets and capability URLs | Endpoints a request without a session may call without a device credential. A list given here replaces the built-in one whole. See [Who a request counts as](#who-a-request-counts-as). |
| `device_puzzle_bits` | `20` | How hard earning a device credential is: about a second or two on a mid-range phone at 20. Each more doubles it; at most 32. |

## Who a request counts as

- **Signed in:** a request with a valid session counts toward its user's limits only. People
  signed in behind one address (an office, a school, a carrier-grade NAT) never share a budget.
- **Not signed in:** a request without one counts toward its address's limits (`ip`) and its
  device's (`device`). Every endpoint callable without signing in, except those `device_exempt`
  lists, asks for a device credential, which an Aspen app earns by solving a small puzzle once
  per deployment. So a crowd at one address is many devices, each with a device's limits, while
  the address keeps a ceiling high enough for a large event.
- **Failures:** sign-in, token refresh, password resets, and registration invites also count what
  proves wrong from one address (`ip_failures`): wrong passwords and codes, unknown refresh tokens
  and invites. A crowd mostly gets these right, so the address's attempts can be generous while a
  machine guessing is soon held.
- **Made-up tokens:** session and bot tokens are signed, so a made-up one is refused without
  reading the database and counts toward nothing. Bot tokens made before tokens were signed can
  only be checked by a lookup, so each address may present a limited number of those that no bot
  holds (`ip_rejected_tokens`). Resetting a bot's token gives it a signed one.

A tool that calls the API without a session must present a device credential as the
`Aspen-Device` header. It earns one with `POST /api/v1/auth/device-challenges` and
`POST /api/v1/auth/device-credentials`. Without one it is refused with `401`
`deviceCredentialRequired`. Load balancers and health checks should ask `GET /api/v1/health`
or `/health/ready`, which ask for none. While a suspension of the limits
(`aspen-chat-server limits suspend`) covers a network, as for a load test, requests from it may
go without one.

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

Registration (`POST /users`) is limited per device, per address, and for everyone together. One
device may make five accounts an hour. One address may make 1000 an hour, all of them at once, so
everyone at a large event or behind one carrier-grade NAT can register together. Everyone together
may make 600 accounts at once, then twenty a second (72000 an hour). That passes a launch's rush.
A deployment expecting more at once raises it:

```toml
[rate_limits.endpoints."POST /users"]
global = { requests = 6000, per_seconds = 60, burst = 3000 }
```

The device and address limits hold back one machine, which must solve a puzzle for each device it
claims to be. Anyone with many addresses is held back by the global limit, unless the deployment
requires a registration invite or a verified email address (see
[Deployment settings](deployment-settings.md)).

## Limits for one endpoint

Limits for one endpoint go under its method and path:

```toml
[rate_limits.endpoints."POST /channels/{channel}/messages"]
user = { requests = 10, per_seconds = 10, burst = 5 }
```

A limit that names an endpoint, or a path parameter, that does not exist stops the server at
startup, with a message saying which.
