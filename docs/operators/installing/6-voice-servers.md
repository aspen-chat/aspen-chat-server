# Step 6: Voice servers

Each voice server is its own machine (or container) with its own `voice_server.toml`.

## Register it

1. On the machine that will run it, measure its capacity:

   ```
   voice_server estimate-capacity
   ```

   This measures the CPU, reads the machine's memory, network, and port range, and prints a
   capacity.

2. Where an API server runs, register it:

   ```
   aspen-chat-server voice-servers add voice-1 --url https://voice-1.chat.example.org --capacity 120
   ```

   It may be run again with the same arguments, so a deployment script can run it every time.
   The dashboard registers servers too.

3. Note the id it prints. It prints the id, the line to put in `voice_server.toml`, and the
   subjects its NATS user's permissions must name, like this:

   ```
   voice-1  https://voice-1.chat.example.org  capacity 120  id 01a11de4-6cd9-7023-a882-a62812d5d6b9

   Give the voice server this id in voice_server.toml (or ASPEN_VOICE_SERVER_ID):

       id = "01a11de4-6cd9-7023-a882-a62812d5d6b9"

   If it signs in to NATS as a user of its own, that user's permissions name the same id (docs/operators/installing/6-voice-servers.md):

       publish:   aspen.voice.report.*.01a11de4-…, aspen.voice.speaking.*.01a11de4-…
       subscribe: aspen.voice.command.01a11de4-…, _INBOX_voice.01a11de4-….>
   ```

### The id

- The id is how the deployment tells its voice servers apart. The voice server reports as that
  id, join tokens name it, and its NATS user (below) may publish only on subjects naming it.
- `voice-servers list` shows every server's id again. So does the dashboard's Server fleet tab,
  with the **ID wizard** on in the app's settings, or whenever a voice server reports under an id
  that is not registered.
- The id never changes for as long as the server stays registered, so it is written once.
- **A server removed and registered again gets a new id.** Its `voice_server.toml` and NATS user
  must be given it.

## Configure it

Give the voice server its id and NATS, in `voice_server.toml`:

```toml
id = "…"
nats_url = "nats.internal:4222"
listen_addr = "127.0.0.1:9000"

[nats_user]
user = "voice-1"
password = "…"

# The TLS proxy in front (below) is on this machine. Without this every client has the
# proxy's address, and the limits each address has apply to everyone together.
[rate_limits]
trusted_proxies = ["127.0.0.1"]

[rtc]
announced_address = "203.0.113.10"
min_port = 40000
max_port = 40999
```

## Give it a NATS user

Give each voice server a NATS user of its own that may do no more than a voice server does.
**Do not give it the token the API servers use:** whoever holds that token can read and write
every event of the deployment, and a voice server runs on a machine people send media to.

NATS then signs everyone in as a user, so the API servers need one too. In `aspen.toml`, replace
`nats_auth_token` with:

```toml
[nats_user]
user = "aspen"
password = "…"
```

Start NATS with a configuration naming both. Add one entry like `voice-1` for each voice server,
with its own id in place of `VOICE_SERVER_ID`:

```
jetstream {}
authorization {
  users = [
    { user: "aspen", password: "…" }
    { user: "voice-1", password: "…", permissions: {
        publish: { allow: [
          "aspen.voice.report.*.VOICE_SERVER_ID",
          "aspen.voice.speaking.*.VOICE_SERVER_ID",
          "aspen.voice.token-key",
          "$JS.API.INFO",
          "$JS.API.STREAM.INFO.KV_aspen_rate_limits",
          "$JS.API.CONSUMER.CREATE.KV_aspen_rate_limits",
          "$JS.API.CONSUMER.CREATE.KV_aspen_rate_limits.>",
          "$JS.API.CONSUMER.DELETE.KV_aspen_rate_limits.>",
          "$JS.API.DIRECT.GET.KV_aspen_rate_limits.>",
          "$JS.API.STREAM.MSG.GET.KV_aspen_rate_limits",
          "$JS.FC.KV_aspen_rate_limits.>"
        ] }
        subscribe: { allow: [
          "aspen.voice.command.VOICE_SERVER_ID",
          "_INBOX_voice.VOICE_SERVER_ID.>"
        ] }
    } }
  ]
}
```

These permissions let the voice server:

| Permission | Lets it |
| --- | --- |
| `aspen.voice.report.{lane}.{id}`, `aspen.voice.speaking.{lane}.{id}` | Publish its own reports. |
| `aspen.voice.command.{id}` | Receive its own commands. |
| `KV_aspen_rate_limits` subjects | Follow a suspension of rate limits (the key-value bucket `aspen_rate_limits`). |
| `aspen.voice.token-key` | Ask the API servers for the public half of the key they sign join tokens with. |
| `_INBOX_voice.{id}` | Receive the replies to its own requests. It asks for them under this prefix, not NATS's shared `_INBOX`. |

A voice server still given `nats_auth_token` works, and warns at startup.

### The join token key

- The first API server to start makes the key and keeps it in the database. No secret needs
  copying to a voice server.
- A voice server can check the tokens that let people into calls, but never make one.
- A voice server that cannot reach an API server at startup keeps asking every few seconds, and
  turns joins away until one answers.

### Reports from voice servers

The API servers apply a report only when:

- it came on a subject naming the server it is about, and
- the call or channel it is about is that server's.

So a voice server taken over can misreport its own calls and no one else's.

It can still cause some trouble beyond its own calls. NATS lets a request name any subject for
its reply, so it can have NATS publish its own answers on the API servers' subjects:

- making every API server reload its plugins or redo background passes;
- replacing a suspension of rate limits with a value that is not read.

It cannot make anything it says believed that way. The API servers refuse to answer anywhere but
the asking server's own inbox, and log `refused to answer a request for the join token key` when
one tries. **That log line means that voice server is compromised.** Firewalling NATS from
everything but the deployment's own machines remains the main defence (see
[Network exposure](network-exposure.md)).

## Open its ports

Clients reach a voice server in three ways, and all must be open to them:

| Traffic | Protocol and port | Notes |
| --- | --- | --- |
| Signalling and the latency check | HTTPS: `GET /health` and the WebSocket `GET /ws` | Through a TLS proxy (below). |
| Media | UDP (and TCP where UDP is blocked), `[rtc] min_port` to `max_port` | Clients send media to `announced_address`. |
| File transfers | UDP `[transfer] port` (3478) | STUN, and the TURN relay. |

### Signalling

The voice server speaks plain HTTP on `listen_addr`. Put a TLS proxy in front of it at the `url`
you registered:

- pass WebSocket upgrades through,
- set `X-Forwarded-For`,
- list the proxy's address in `[rate_limits] trusted_proxies`.

The server warns at startup when it listens on loopback with no trusted proxies.

### Media

`announced_address` is the address clients send media to.

- Behind NAT, set it to the server's public address.
- Leave it out only when the machine has a public address on an interface.
- **Never set it to a loopback address.**

### File transfers

The transfer port carries STUN, so two people's devices can connect directly, and the TURN relay
for transfers that go through the server. The relay carries no more than `[transfer] relay_mbps`
(50) in all. Set `relay_mbps = 0` not to relay transfers at all.

## Check that it is registered

When a voice server starts, it asks the API servers for the key join tokens are signed with,
naming its id. They answer whether that id is registered.

- If it is not, the voice server logs `this voice server's id is not registered with the
  deployment` with the id, and stops with an error. Fix its `id` and start it again.
- When no API server is up yet, it keeps asking, and the check happens once one answers.

The API servers watch for the same mistake from their side. A voice server whose id is not
registered sends load reports that match no server, so they are dropped, and the server it was
meant to be shows as **Silent** in the dashboard. While such reports keep arriving (from a voice
server already running when it was removed, say):

- each API server logs `a voice server whose id is not registered is reporting` with the id;
- the Server fleet tab lists it under **Voice servers that are not registered**, beside every
  registered server's id to copy.

## When a voice server stops being offered

- A voice server that stops reporting for a minute is no longer offered to people joining calls.
- One that people fail to reach is suspended for `failure_window_seconds` after
  `failure_threshold` of them try in that time, unless it is the last one taking calls.

---

Previous: [Step 5: Serving the web client](5-web-client.md) · Next:
[Step 7: The first administrator](7-first-administrator.md)
