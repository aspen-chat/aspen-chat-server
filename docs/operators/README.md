# Running Aspen

This guide is for whoever runs an Aspen deployment: a community of a dozen people on a spare
machine, or millions across a fleet. Nothing in it assumes you have read the code.

- [Installing](installing.md): what an Aspen deployment is made of, building it, starting it,
  TLS, serving the web client, voice servers, and the first administrator.
- [Configuration](configuration.md): every setting of `aspen.toml` and `voice_server.toml`.
- [Federation](federation.md): letting your users use other deployments, and theirs yours.
- [Backups](backups.md): what to keep, what can be lost, and restoring.
- [Troubleshooting](troubleshooting.md): what each error people see means and what to do,
  and what to check when something does not work.

## What a deployment is made of

| Part | What it does | Keeps anything? |
| --- | --- | --- |
| `aspen-chat-server` | The API, the event stream, the passkey page, and the federation document. Run as many as you need behind a load balancer. | No: everything is in the services below. |
| `voice_server` | Calls: forwards each participant's audio and video to the others. Run one or more, each registered with the API servers. | No. |
| PostgreSQL | Every account, community, message, and setting, and this deployment's federation key. | **Yes: back it up.** |
| Object storage (S3 compatible) | Attachments, icons, avatars, and link preview images. | **Yes: back it up.** |
| NATS with JetStream | Carries events between servers. Keeps the last minute of them in memory, and the voice servers' reports until an API server has applied them. | No. |
| Valkey | Rate limit counters, presence, and short-lived sign-in state. | No: losing it signs no one out and loses nothing but a few minutes of counters. |
| The web client | A static site: `pnpm build` in `client/`. The desktop and mobile apps need no hosting. | No. |

Every server reads its settings from a file in its working directory (`aspen.toml` for the API
server, `voice_server.toml` for a voice server), and environment variables override the file.
Both files hold secrets: keep them readable only by the account that runs the server.
