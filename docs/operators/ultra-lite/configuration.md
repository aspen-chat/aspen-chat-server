# Ultra Lite's configuration

`aspen-ul init` writes `/var/lib/aspen-ul/aspen-ul.toml`. Edit it, then restart the deployment
(`sudo systemctl restart aspen-ul`). Everything a full deployment's servers would be told of the
database, the bus, and the voice server, Ultra Lite works out itself.

## The deployment

| Setting         | Default                  | What                                                                                                                                                                                 |
| --------------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `public_url`    | (required)               | The deployment's address, `https://` and the domain. **It can never change.**                                                                                                        |
| `media_url`     | `https://media.<domain>` | Where uploaded files are served. It must be a name of its own, never `public_url`'s.                                                                                                 |
| `listen`        | every address            | The addresses listened at, as a list (`["192.0.2.7"]`).                                                                                                                              |
| `port`          | `443`                    | The port HTTPS is served at.                                                                                                                                                         |
| `redirect_port` | `80`                     | The port plain HTTP is answered at while TLS is on: a page is sent on to HTTPS, and the API refuses it, since a request sent that way crossed the network unencrypted. `0` for none. |
| `memory_mib`    | the machine's            | How much memory the deployment may use, in MiB, which sizes its database and what it holds in memory. Give it when the machine runs other things too.                                |

## `[tls]`

`mode` is one of:

| `mode`  | What                                                                                                                                                                                                                                                                                                     |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `acme`  | A certificate from Let's Encrypt for both names, renewed while the deployment runs. It is validated at port 443, so 443 must reach the machine. `email` is where Let's Encrypt writes; `directory` names another ACME authority's directory URL, and `ca_file` the authorities to trust for reaching it. |
| `files` | A certificate and key you keep: `cert` and `key`, PEM files naming both names. Restart the deployment after replacing them.                                                                                                                                                                              |
| `off`   | Plain HTTP, for a proxy on the same machine that serves HTTPS at both names and passes requests to `port` on `127.0.0.1`, with WebSocket upgrades. `init --tls off` listens on loopback alone.                                                                                                           |

## `[voice]`

| Setting             | Default          | What                                                                                                                                    |
| ------------------- | ---------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `announced_address` | the machine's    | The addresses calls are offered at, as a list. Behind a router, the router's public addresses; `aspen-ul doctor` says when it is wrong. |
| `media_port`        | `40000`          | Calls' media, UDP and TCP.                                                                                                              |
| `transfer_port`     | `3478`           | STUN and TURN for file transfers in calls, UDP and TCP.                                                                                 |
| `relay_ports`       | `[40100, 40199]` | The UDP ports relayed file transfers take.                                                                                              |
| `capacity`          | `100`            | The most people in calls at once.                                                                                                       |

## `[server]`: everything else

Any other setting of the full server's `aspen.toml` (see [Configuration](../configuration/index.md))
goes under `[server]`, as it would be written there: `[server.email]` for mail,
`[server.rate_limits]`, `[server.media.previews]`, and the rest. The settings Ultra Lite works out
itself are refused there: `public_url`, `database_url`, `database_pool_size`, NATS's, Valkey's,
`[server.web_client]`, and `[server.media.directory]`.

To keep uploaded files in S3-compatible object storage rather than on the machine, give
[`[server.media.s3]`](../configuration/media.md). `media_url` and its name are then not used, and
backups no longer hold the files.
