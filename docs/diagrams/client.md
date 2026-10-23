# Client packages

`client/` is a pnpm workspace: one TypeScript web app, shipped to browsers as it is, to desktops
inside Electron, and to phones inside Capacitor. `client/AGENTS.md` holds the rules that keep the
packages apart.

## Packages and shells

```mermaid
flowchart TB
    protocol["@aspen/protocol<br/>packages/protocol: generated types, HTTP and session client,<br/>event stream, sync and store, voice and file transfers<br/>(no React, no platform API)"]
    app["@aspen/app<br/>packages/app: React and React Aria UI, built by Vite"]
    dist[/"packages/app/dist<br/>the built web client"/]

    subgraph shells["Shells"]
        server["aspen-chat-server<br/>serves dist at public_url"]
        desktop["@aspen/desktop<br/>packages/desktop: Electron main and preload"]
        mobile["@aspen/mobile<br/>packages/mobile: Capacitor"]
    end

    capture["aspen-capture<br/>desktop/native/capture (Rust):<br/>game and window capture"]
    ios["iOS app<br/>mobile/ios, with AspenNotificationService"]
    android["Android app<br/>mobile/android, with its push handler"]

    app -- "workspace dependency" --> protocol
    app -- "vite build" --> dist
    dist --> server
    dist -- "loadFile; window.aspenDesktop<br/>through the preload bridge" --> desktop
    dist -- "pnpm sync copies it in (webDir)" --> mobile
    desktop -- "spawns" --> capture
    mobile --> ios & android
```

- `@aspen/app` is the only package that depends on another; the shells import nothing from it and
  carry its build instead, so the page is the same everywhere. What it needs of a host goes through
  the desktop's preload bridge (`window.aspenDesktop`) or Capacitor's plugins, never a Node or
  Electron API. See [Web client](../architecture/web-client/index.md) for how the server serves it.
- `@aspen/protocol` stays free of React and platform APIs so it can be tested in Node and used by
  any shell.
- The iOS notification service extension and Android's handler decrypt Web Push themselves, while
  the app is not running. See [Push](../architecture/push.md) and
  `client/docs/architecture/push.md`.

## Schemas join the server and the client

The client never declares a server type by hand. Each server writes the schemas of what it sends
and accepts, and `pnpm codegen` turns them into `@aspen/protocol`'s generated types, so a change on
the server surfaces as a compile error in the client.

```mermaid
flowchart LR
    enum["message_enum.rs<br/>(server/wire)"]
    mg["message_gen"]
    wire["aspen_wire<br/>records, requests, events"]
    api["aspen_api<br/>utoipa paths, event stream types"]
    vp["voice_protocol::signal"]

    chat["aspen-chat-server<br/>--gen-openapi-schema"]
    vs["voice_server<br/>--gen-signal-schema"]

    openapi[/"openapi.yaml"/]
    events[/"event_schema.json"/]
    federation[/"federation_schema.json"/]
    signal[/"voice_signal_schema.json"/]

    codegen["pnpm codegen<br/>(packages/protocol/scripts/codegen.ts)"]
    generated["@aspen/protocol<br/>src/generated/"]

    enum --> mg --> wire --> api --> chat
    vp --> vs
    chat --> openapi & events & federation
    vs --> signal
    openapi & events & signal --> codegen --> generated
```

- The four files are written to the repository root and are gitignored; `pnpm codegen:regen`
  regenerates them from the servers before generating.
- `openapi.yaml` becomes the types the openapi-fetch client is typed with; `event_schema.json` is
  copied verbatim for the event stream's runtime validator; `voice_signal_schema.json` types the
  voice server's signalling frames.
- `federation_schema.json` describes what deployments exchange with each other, which no client
  reads (see [Federation](../architecture/federation/index.md)).
