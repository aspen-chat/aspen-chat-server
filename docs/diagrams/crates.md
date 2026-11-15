# Rust crates

The root `Cargo.toml` is one workspace of every server-side crate. Each arrow below is a dependency
in a crate's `Cargo.toml`, pointing from the crate that depends to the crate it depends on.

## The API server

`aspen-chat-server` is split into three layers, `api` over `app` over `schema`, with crates beneath
`app` that need nothing of it (see Layers in `AGENTS.md`). `api` never reaches past `app` into the
database; `app` never depends on `api`.

```mermaid
flowchart LR
    bin["aspen-chat-server<br/>server/: main, connections,<br/>operator commands"]
    api["aspen_api<br/>server/api: HTTP handlers,<br/>event stream"]
    app["aspen_app<br/>server/app: business logic,<br/>permissions, events"]

    subgraph beneath["Beneath app: needs nothing of it"]
        database["aspen_database<br/>PostgreSQL over TLS, the pool"]
        previews["aspen_previews<br/>pictures, SVGs, and posters"]
        svg_protocol["aspen_svg_render (protocol alone)<br/>what the SVG renderer speaks"]
        search_text["aspen_search_text<br/>the words a message is searched by"]
        plugin_rt["aspen_plugin_runtime<br/>wasmtime, call limits"]
        link_preview["aspen_link_preview"]
        outbound["aspen_outbound<br/>requests to other hosts"]
        webpush["aspen_webpush<br/>RFC 8291 and 8292"]
        fed_core["aspen_federation_core<br/>JWS, gates' policy"]
        email_tpl["aspen_email_templates"]
        ephemeral["aspen_ephemeral<br/>short-lived shared state:<br/>keys, buckets, mana (Valkey)"]
    end

    subgraph foundation["What every layer speaks"]
        wire["aspen_wire<br/>records, requests, events, ids"]
        locale["aspen_locale<br/>catalogue, t!"]
        schema["aspen_schema<br/>generated Diesel schema"]
        message_gen["message_gen<br/>proc macro"]
    end

    shared["The shared crates<br/>(see Every binary)"]

    bin --> api --> app
    bin --> app
    app --> database & previews & search_text & plugin_rt & link_preview & webpush & fed_core & email_tpl & ephemeral
    link_preview --> outbound
    previews --> svg_protocol
    plugin_rt & fed_core & link_preview --> wire
    api & app --> wire & locale
    app --> schema
    wire --> message_gen & schema & locale
    bin & app --> shared
```

- The binary also names `aspen_schema` and `aspen_wire` itself, for its operator commands; those
  edges are left out to keep the layers readable.
- Which shared crate each of these uses is drawn in [Every binary](#every-binary). Besides the
  binary and `app`, `api` uses `aspen_limits` and `aspen_metrics`, and `aspen_database` uses
  `aspen_metrics`.
- `message_gen` turns the one enum in `server/wire/src/message_enum.rs` into each entity's record,
  request bodies, and server events, so `aspen_wire` is where the server and its clients agree.
- `aspen_wire` names `aspen_schema` for the SQL traits of the types it stores (`wire_name_traits!`
  and the rest), which is why the schema sits beneath it rather than only beneath `app`.
- `aspen_previews` names the SVG renderer's crate without its `render` feature, for the protocol
  alone (arguments, exit codes, the pixels it writes, and `fit`); the renderer is the program
  `aspen-svg-render`, which it runs ([Every binary](#every-binary)).
- The crates beneath `app` are re-exported from the `app` module that uses each one
  (`app::database`, `app::push::webpush`, `app::federation::jws`), so a path through `app` names it
  wherever it lives.

## Every binary

Six binaries come out of the workspace. The crates they share are the protocol between the API
server and the voice server, and the plumbing every server needs.

```mermaid
flowchart TB
    subgraph bins["Binaries"]
        chat["aspen-chat-server"]
        voice_server["voice_server<br/>rooms, signalling, STUN and TURN"]
        migrate["aspen-migrate<br/>migrations and their registry"]
        bench["aspen-bench<br/>(crate aspen_bench): load generator"]
        ul["aspen-ul<br/>a deployment in one program"]
        svg_render["aspen-svg-render<br/>draws one SVG behind its sandbox"]
    end

    app["aspen_app<br/>(and everything beneath it)"]
    database["aspen_database"]
    search_text["aspen_search_text"]
    voice_protocol["voice_protocol<br/>join tokens, control, signalling"]
    voice_media["voice_media<br/>media threads, the synthetic peer"]
    str0m[/"str0m (crates.io)<br/>WebRTC, crypto through AWS-LC"/]
    resvg[/"resvg (crates.io)<br/>parsing and drawing SVGs"/]
    seccompiler[/"seccompiler (crates.io)<br/>its seccomp filter"/]
    voice_client["voice_client<br/>the mobile apps' call engine"]
    limits["aspen_limits<br/>limit format, GCRA, trusted proxies"]
    metrics["aspen_metrics<br/>Prometheus exporter, jemalloc stats"]
    sockets["aspen_sockets<br/>dual-stack listeners"]
    tls["aspen_tls<br/>TLS files, NATS connections"]
    bus["aspen_bus<br/>the message bus: subjects,<br/>streams, buckets"]
    config_values["aspen_config_values<br/>lists and tables in settings"]
    bench_protocol["aspen_bench_protocol<br/>seeding and suspension messages"]

    chat --> app & voice_protocol & limits & bus & metrics & sockets & bench_protocol
    ul --> chat & voice_server & migrate & app & bus & sockets
    app --> database & voice_protocol & tls & bus & config_values & bench_protocol
    voice_server --> voice_protocol & voice_media & limits & bus & metrics & sockets & tls & config_values
    migrate --> database & search_text
    bench --> voice_protocol & voice_media & limits & bus & metrics & bench_protocol
    voice_media --> voice_protocol & sockets & str0m
    svg_render --> resvg & seccompiler
    voice_client --> voice_protocol
    voice_protocol --> limits
    limits --> bus
    database --> metrics
```

- `voice_protocol` is the whole contract between the two servers: the join token the API server
  signs and the voice server checks, the reports and commands they exchange over NATS, and the
  signalling frames clients exchange with the voice server.
- `voice_media` is the voice server's media: threads holding str0m connections, forwarding between
  them, and filling the audio slots. Its `peer` feature is a synthetic str0m client, which
  `voice_server`'s `estimate-capacity` and the bench's simulated participants run on. str0m is drawn
  because it is the one crate outside the workspace the voice server's media is built on; see
  [Voice](../architecture/voice/voice-server.md#media-threads).
- `aspen-svg-render` needs nothing of the workspace: the API server (and `aspen-ul`, which carries
  it) runs it as a program, once for each SVG, under resource limits, and it installs a seccomp
  filter on itself before reading the SVG. resvg and seccompiler are drawn as the crates outside the
  workspace it is built on. See [SVGs](../architecture/attachment-previews/svgs.md).
- `voice_client` is the call engine the mobile apps run natively, shared by both: it drives each
  platform's WebRTC, HTTP, and WebSockets through interfaces the iOS and Android layers implement,
  and speaks `voice_protocol`'s signalling frames to the voice server, which is why it depends on
  that crate alone. No crate depends on it yet; it is not a binary of the workspace but a library
  the apps link.
- `aspen-migrate` needs the database connection, and `aspen_search_text` for the migration that
  found the words of every message written before `search_text` existed, at the version the server
  finds them with; its migrations are its own (`migrate/src/migrations/`), and `aspen_schema` is
  regenerated from the database they make.
- `aspen-bench` drives a deployment through its public API and the voice servers, and reads both
  servers' metrics by the names `aspen_metrics` gives them. The population it plans is written into
  the database by `aspen-chat-server bench seed`; `aspen_bench_protocol` is the plan and the
  manifest the two exchange. See [Benchmarking](../architecture/benchmarking.md).

## Outside the workspace

Some Rust is built on its own, for another target or because it links what the servers must not
need.

```mermaid
flowchart LR
    wit[/"spec/plugin.wit<br/>the plugin interface"/]
    plugin_rt["aspen_plugin_runtime<br/>(host side, wasmtime)"]
    subgraph plugins["Example plugins: wasm32-wasip2, a workspace each"]
        word_filter["plugins/word_filter"]
        forum["plugins/forum"]
        calendar["plugins/calendar"]
        blackjack["plugins/blackjack"]
    end
    capture["aspen_capture<br/>client/packages/desktop/native/capture<br/>libobs, PipeWire, ScreenCaptureKit"]
    desktop["@aspen/desktop"]
    voice_server["voice_server"]

    plugin_rt -- "wasmtime bindgen" --> wit
    word_filter & forum & calendar & blackjack -- "wit-bindgen" --> wit
    desktop -- "spawns" --> capture
    capture -- "WebRTC capture connection<br/>(str0m, through the desktop's offers)" --> voice_server
```

- Plugins and the server share nothing but `spec/plugin.wit`; a plugin is a WebAssembly component
  the operator installs, and `aspen_plugin_runtime` runs it. See
  [Plugins](../architecture/plugins/index.md).
- `aspen_capture` is the desktop app's helper process for game and window capture, built by
  `pnpm build:native` in `client/packages/desktop`, since it links libobs. It sends to the voice
  server as a str0m WebRTC peer of its own (the capture connection), answering the offers the
  desktop app passes it, its DTLS and SRTP through str0m's Rust crypto. See
  [Client packages](client.md).
