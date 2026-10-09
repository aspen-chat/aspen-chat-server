# Voice calls

How the API servers choose voice servers for calls, and registering the voice servers. A voice
server's own settings are on [Voice server](voice-server.md).

## `[voice]`

| Setting | Default | |
| --- | --- | --- |
| `failure_threshold` | `5` | How many different people failing to reach a voice server, within `failure_window_seconds`, suspend it. See [Suspending a failing server](#suspending-a-failing-server). |
| `failure_window_seconds` | `3600` | How long failures are counted, and how long a suspension lasts. |
| `join_token_ttl_seconds` | `60` | How long someone has to reach a voice server after asking to join. |
| `candidate_limit` | `10` | The most voice servers one person is offered to choose the nearest from. |
| `offer_silence_seconds` | `60` | A voice server that has not reported for this long is not offered to people joining. |
| `session_silence_seconds` | `86400` | A voice server that has not reported for this long has its calls ended. |
| `idle_session_seconds` | `86400` | A call that never had two people in it at once ends after this long. |

### Suspending a failing server

- A server is suspended when `failure_threshold` different people fail to reach it within
  `failure_window_seconds`.
- The suspension lasts `failure_window_seconds`. The server then takes calls again on its own.
- An administrator enabling it ends the suspension sooner.
- The last server taking calls is never suspended.

Who counts as a failure:

- Only this deployment's people whom a join offer sent to that server within the token's
  lifetime and a minute.
- Not those who joined a call there within the window.
- Never bots, and never people from other deployments.

## Voice servers

The voice servers calls run on are registered in the database. Holders of Manage voice servers
add, change, disable, and remove them in the Administration Dashboard. From the terminal:

| Command | What it does |
| --- | --- |
| `aspen-chat-server voice-servers add NAME --url URL --capacity N` | Registers one, or gives the one already registered by that name this address and capacity. A deployment script may run it every time it deploys. It prints the server's id; see [The voice server's id](#the-voice-servers-id). |
| `voice-servers set NAME [--url URL] [--capacity N] [--enabled true\|false]` | Changes one. A disabled server is offered to no one joining a call; calls already on it go on. |
| `voice-servers remove NAME` | Removes one that holds no calls. Disable it first and let its calls end, or remove it from the dashboard, which ends them. |
| `voice-servers list` | Lists them, with their ids. |

### `url` and `capacity`

- `url` is where clients reach the voice server, such as `https://voice-1.chat.example.org`.
- It is an `http` or `https` address.
- It must be `https` wherever `public_url` is, here and in the dashboard.
- `capacity` is the most people it carries at once. `voice_server estimate-capacity` suggests
  one.

### The voice server's id

`voice-servers add` prints the server's id. The voice server's `id` must be that id, and so must
the subjects its NATS user's permissions name (see
[Step 6: Voice servers](../installing/6-voice-servers.md#configure-it)).
