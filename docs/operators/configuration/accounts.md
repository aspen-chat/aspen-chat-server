# Accounts and presence

Sign-in work, per-account limits, and when people show as away.

## `[auth]`

| Setting                         | Default     |                                                                                                         |
| ------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------- |
| `reverify_seconds`              | `600`       | Changing security settings needs a password or code given within this long.                             |
| `password_hashing_threads`      | one per CPU | How many passwords may be hashed or checked at once. Each takes 19 MiB and most of a core for a moment. |
| `password_hashing_wait_seconds` | `10`        | How long a sign-in waits for one of those threads before it is refused with `serverBusy`.               |
| `android_apps`                  | none        | Android apps, besides the official one, that may use the deployment's passkeys. See below.              |

### Android apps and passkeys

The Android app uses a deployment's passkeys itself, through Android's Credential Manager, when the
deployment names it in `/.well-known/assetlinks.json`. Otherwise it hands each passkey prompt to the
phone's browser. The deployment names the official app's release signing certificates on its own.
None exists yet, so today every build is one you name yourself: a fork, or a build signed with your
own key, debug builds included.

Name each as its package and the SHA-256 fingerprint of the certificate it is signed with:

```toml
[auth]
android_apps = ["com.aspen_chat.client=FA:C6:17:45:DC:09:03:78:6F:B9:ED:E6:2A:96:2B:39:9F:73:48:F0:BB:6F:89:9B:83:32:66:75:91:03:3B:9C"]
```

or in the environment, separated by commas:
`ASPEN_AUTH__ANDROID_APPS="com.aspen_chat.client=FA:C6:…,org.example.fork=12:34:…"`.

- Read the fingerprint with `apksigner verify --print-certs app.apk` or
  `keytool -list -v -keystore <keystore>`; colons are optional, and case does not matter. A debug
  build's key is `~/.android/debug.keystore`
  (`keytool -list -v -keystore ~/.android/debug.keystore -storepass android`).
- A package may be named several times, once per certificate.
- Nothing is named where the deployment offers no passkeys (an `http` `public_url` at another host
  than `localhost`).
- Google reads `https://<public_url's host>/.well-known/assetlinks.json` from the internet, at port
  443, before a phone lets the app use the deployment's passkeys. So a deployment whose `public_url`
  names another port, or that Google cannot reach (one on a private network), is never confirmed,
  and the app hands its prompts to the browser. Google keeps what it read for a while, so a change
  can take a day to reach phones.

## `[limits]`

| Setting                         | Default                      |                                                                                                                                     |
| ------------------------------- | ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `max_communities_per_user`      | `500`                        | The most communities one account may belong to.                                                                                     |
| `max_event_streams_per_user`    | `20`                         | The most live connections (event streams) one account may hold open on each API server. Each window or device of the app holds one. |
| `max_event_streams_per_address` | `200`                        | The most live connections one client address may hold open on each API server before they sign in. See below.                       |
| `max_identifying_event_streams` | half of `database_pool_size` | The most live connections each API server signs in at once. See below.                                                              |
| `max_followed_dm_partners`      | `100`                        | The most DM partners whose profile changes one live connection is told of. See below.                                               |

- One event stream more than either of the first two limits is closed with code 4429.
- `max_event_streams_per_address` counts an IPv6 address by its `[rate_limits] ipv6_prefix` network.
  A connection that has signed in counts toward `max_event_streams_per_user` instead.
- Raise `max_event_streams_per_address` when so many people reach the deployment from one address at
  once that some are closed before they sign in: behind one NAT, or one proxy you have not listed in
  `trusted_proxies`.
- `max_followed_dm_partners` bounds how many people's profile changes (a new name or picture, a
  deleted account) one connection hears of without sharing a community with them: the people in the
  account's most recently active DMs, as many DMs as keep them within the limit. Someone from an
  older DM is shown as last read until the app reads them again. Each is one entry in the server's
  memory per connection; raise it for a deployment whose people keep many DMs open at once.
- When many people connect at once (a server restarting, an ISP's customers coming back from an
  outage), each API server signs in at most `max_identifying_event_streams` at a time, and turns new
  ones away while requests already queue for a database connection, so those already connected stay
  served. A connection turned away is closed with code 1013, and the app comes back by itself ten to
  twenty seconds later. `aspen_event_stream_connects_total{outcome="busy"}` counts them. Raise it
  only with `database_pool_size`.

## `[presence]`

| Setting              | Default |                                                                                              |
| -------------------- | ------- | -------------------------------------------------------------------------------------------- |
| `away_after_seconds` | `600`   | How long someone connected but not using Aspen stays shown as online before showing as away. |

Clients report use at most once a minute, so a value much less than a few minutes makes people
flicker between online and away.
