# Voice servers and the registry

Each voice server is a row of the `voice_server` table. The API servers offer, command, and suspend servers by that row; the voice server learns its own row from `id` in `voice_server.toml` (see [The voice server process](voice-server.md)).

## Addresses

- A server is registered at an `http` or `https` address.
- It must be `https` wherever `public_url` is. `check_url` enforces this on every path that sets an address.

## Managing servers

| Route | How |
|---|---|
| REST | `/voice-servers`, with the Manage voice servers permission |
| Terminal | `aspen-chat-server voice-servers list\|add\|set\|remove` |

- `add` registers a server, or updates the one of that name. A deployment's scripts may run it every time.
- `remove` refuses a server that holds calls. Ending them takes a server's context.
- Removing a voice server clears its waiting reports (see [Reports](voice-reports.md)).
- Enabling or disabling a server ends a suspension; enabling it also forgets its failures (see [Choosing a server](server-selection.md)).
- Sessions on a removed server end with reason `serverRemoved` (see [Sessions](sessions.md)).

## Columns shown to clients

| Column | Wire field | Meaning |
|---|---|---|
| `suspended_until` | `suspendedUntil` | When a suspension for failure reports ends |

The `capacity` a server is registered with comes from `voice_server estimate-capacity` (see [The voice server process](voice-server.md#capacity)).

## The shared crate

`voice_protocol/` is what the API server and the voice servers share:

- the join token (`voice_protocol::token`),
- the control messages, reports and commands (`voice_protocol::control`),
- the signalling frames (`voice_protocol::signal`).

## Commands to a voice server

Commands from the API server to a voice server (mute, kick, grant, close, end sign-ins) travel on `aspen.voice.command.{server}`. Each `VoiceCommand` is described where it is used: [rechecks](rechecks.md), [moderation](moderation.md), and [duplicate rooms](sessions.md#duplicate-rooms).
