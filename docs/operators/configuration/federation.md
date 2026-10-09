# Federation settings

See [Federation](../federation/index.md) for what these mean together. The deployment's domain is
`public_url`'s host. The federation gates are [deployment settings](deployment-settings.md).

## `[federation]`

| Setting | Default | |
| --- | --- | --- |
| `standing_interval_seconds` | `3600` | How often this deployment asks other deployments whether their users here are still in good standing. It also reads again the documents of the deployments it federates with that are in use, which is how soon it notices one replaced its key. |
| `standing_grace_seconds` | `86400` | How long another deployment may go unreached before its users' sessions here end. |
| `max_arrivals_per_home_per_day` | `500` | The most people and bots of one other deployment who may sign in here for the first time in one day (UTC). Those over it are refused with `federationRefused` until the next day. |

## `[federation.development]`

For running deployments side by side on one machine (`scripts/dev_federation.py`).

**A server whose `public_url` is not at `localhost`, a name under it, or a loopback address
refuses to start with either setting given.**

| Setting | Default | |
| --- | --- | --- |
| `extra_root_certificates` | `[]` | PEM files of certificate authorities to trust, besides the system's, when calling other deployments. |
| `allow_private_addresses` | `false` | Lets this server call deployments at private and loopback addresses, and push to push services at loopback addresses (`scripts/dev_push.py`). Pushes never go to other private addresses. |
