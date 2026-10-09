# Accounts and presence

Sign-in work, per-account limits, and when people show as away.

## `[auth]`

| Setting | Default | |
| --- | --- | --- |
| `reverify_seconds` | `600` | Changing security settings needs a password or code given within this long. |
| `password_hashing_threads` | one per CPU | How many passwords may be hashed or checked at once. Each takes 19 MiB and most of a core for a moment. |
| `password_hashing_wait_seconds` | `10` | How long a sign-in waits for one of those threads before it is refused with `serverBusy`. |

## `[limits]`

| Setting | Default | |
| --- | --- | --- |
| `max_communities_per_user` | `500` | The most communities one account may belong to. |
| `max_event_streams_per_user` | `20` | The most live connections (event streams) one account may hold open on each API server. Each window or device of the app holds one. |
| `max_event_streams_per_address` | `200` | The most live connections one client address may hold open on each API server, counted before they sign in. See below. |

- One event stream more than either limit is closed with code 4429.
- `max_event_streams_per_address` counts an IPv6 address by its `[rate_limits] ipv6_prefix`
  network.
- Raise `max_event_streams_per_address` when many people reach the deployment from one address:
  behind one NAT, or one proxy you have not listed in `trusted_proxies`.

## `[presence]`

| Setting | Default | |
| --- | --- | --- |
| `away_after_seconds` | `600` | How long someone connected but not using Aspen stays shown as online before showing as away. |

Clients report use at most once a minute, so a value much less than a few minutes makes people
flicker between online and away.
