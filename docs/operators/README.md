# Running Aspen

This guide is for whoever runs an Aspen deployment, from a single spare machine to several
servers. Nothing in it assumes you have read the code.

## Guides

| Guide | What it covers |
| --- | --- |
| [Installing](installing/index.md) | Step by step: the services, building, configuring, starting, the web client, voice servers, and the first administrator. Then network exposure and upgrading. |
| [Configuration](configuration/index.md) | Every setting of `aspen.toml` and `voice_server.toml`. |
| [Federation](federation/index.md) | Letting your users use other deployments, and theirs yours. |
| [Plugins](plugins.md) | Installing plugins, granting what they ask, and where they run. |
| [Backups](backups.md) | What to keep, what can be lost, and restoring. |
| [Troubleshooting](troubleshooting/index.md) | What each error people see means and what to do, and what to check when something does not work. |

## What a deployment is made of

| Part | What it does | Keeps anything? |
| --- | --- | --- |
| `aspen-chat-server` | The API, the event stream, the web client, the passkey page, and the federation document, all at your deployment's one address. Run as many as you need behind a load balancer. With `--private-worker`, one serves nothing and only shares the background work. | No: everything is in the services below. |
| `voice_server` | Calls: forwards each participant's audio and video to the others. Run one or more, each registered with the API servers. | No. |
| PostgreSQL | Every account, community, message, and setting, and this deployment's federation key. | **Yes: back it up.** |
| Object storage (S3 compatible) | Attachments, icons, avatars, and link preview images. | **Yes: back it up.** |
| NATS with JetStream | Carries events between servers. Keeps the last minute of them in memory, and the voice servers' reports until an API server has applied them. | No. |
| Valkey | Rate limit counters, presence, and short-lived sign-in state. | No: see [Valkey](#valkey). |
| The web client | `pnpm build` in `client/`, served by each API server. The desktop and mobile apps need no hosting. | No. |

### Valkey

Losing Valkey signs no one out, and loses nothing but a few minutes of counters. While it is
unreachable, signing in, password resets, registration, and invites are refused.

## Settings files

Every server reads its settings from a file in its working directory:

- `aspen.toml` for the API server,
- `voice_server.toml` for a voice server.

Environment variables override the file.

**Both files hold secrets.** Keep them readable only by the account that runs the server.
