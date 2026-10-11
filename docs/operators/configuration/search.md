# Search

Searching messages for words costs little when the words are rare, or common near the present.
A search that finds nothing near the present reads on through history until it finds something.
That reading is a full core of the database server for as long as it lasts, so the deployment
limits how much of it runs at once.

## `[search]`

| Setting | Default | |
| --- | --- | --- |
| `deep_concurrency` | `2` | How many such searches read at once, on the whole deployment. **Give every API server the same number.** |
| `round_seconds` | `20` | How long one request reads on before it answers with how far it got. The app then asks again from there. |

- Set `deep_concurrency` to about a quarter of the database server's cores. The default suits a
  small machine.
- Searches waiting for a turn all advance, more slowly the more of them there are. Nobody is
  refused.
- Keep `round_seconds` under the shortest timeout of anything between the apps and this server.
  A reverse proxy commonly cuts a request that has been idle for sixty seconds.

## Watching it

| Metric | Shows |
| --- | --- |
| `aspen_deep_search_rounds_total{outcome="unfinished"}` | Requests that ran out of time while still reading. Many of them, with slow steps, means searches are queueing. |
| `aspen_deep_search_step_duration_seconds` | How long one step takes, its wait for a turn included. A step alone takes well under a second. |

Raise `deep_concurrency` when steps are slow and the database server has cores to spare. Lower
it when searches slow everything else down.
