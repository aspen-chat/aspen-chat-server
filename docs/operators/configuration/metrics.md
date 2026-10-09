# Prometheus metrics

## `[metrics]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Prometheus metrics at `GET /metrics`, on a listener of their own. |
| `listen_addr` | `127.0.0.1:9464` | **Keep it off public interfaces.** |

A voice server has the same section, listening on `127.0.0.1:9465` by default; see
[Voice server](voice-server.md#metrics-and-rate_limits).
