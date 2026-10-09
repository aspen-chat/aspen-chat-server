# Step 8: Checking it works

## The smoke test

```
scripts/smoke_servers.py --bin target/release
```

It needs the services from `docker-compose.yaml`. It:

1. starts a throwaway database and both servers,
2. registers a user,
3. searches,
4. joins a call through the voice server,
5. reads both metrics endpoints,
6. cleans up.

## Metrics

Both servers export Prometheus metrics on loopback:

| Server | Address |
| --- | --- |
| `aspen-chat-server` | `127.0.0.1:9464` |
| `voice_server` | `127.0.0.1:9465` |

Scrape them from the same machine, and keep them off public interfaces.

---

Previous: [Step 7: The first administrator](7-first-administrator.md) · Next:
[Network exposure](network-exposure.md)
