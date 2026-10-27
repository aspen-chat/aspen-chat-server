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
        previews["aspen_previews<br/>pictures and posters"]
        search_text["aspen_search_text<br/>the words a message is searched by"]
        plugin_rt["aspen_plugin_runtime<br/>wasmtime, call limits"]
        link_preview["aspen_link_preview"]
        outbound["aspen_outbound<br/>requests to other hosts"]
        webpush["aspen_webpush<br/>RFC 8291 and 8292"]
        fed_core["aspen_federation_core<br/>JWS, gates' policy"]
        email_tpl["aspen_email_templates"]
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
    app --> database & previews & search_text & plugin_rt & link_preview & webpush & fed_core & email_tpl
    link_preview --> outbound
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
- The crates beneath `app` are re-exported from the `app` module that uses each one
  (`app::database`, `app::push::webpush`, `app::federation::jws`), so a path through `app` names it
  wherever it lives.

## Every binary

Four binaries come out of the workspace. The crates they share are the protocol between the API
server and the voice server, and the plumbing every server needs.

```mermaid
flowchart TB
    subgraph bins["Binaries"]
        chat["aspen-chat-server"]
        voice_server["voice_server<br/>mediasoup SFU, STUN and TURN"]
        migrate["aspen-migrate<br/>migrations and their registry"]
        bench["aspen-bench<br/>(crate aspen_bench): load generator"]
    end

    app["aspen_app<br/>(and everything beneath it)"]
    database["aspen_database"]
    search_text["aspen_search_text"]
    voice_protocol["voice_protocol<br/>join tokens, control, signalling"]
    limits["aspen_limits<br/>limit format, GCRA, trusted proxies"]
    metrics["aspen_metrics<br/>Prometheus exporter, jemalloc stats"]
    sockets["aspen_sockets<br/>dual-stack listeners"]
    tls["aspen_tls<br/>TLS files, NATS connections"]
    config_values["aspen_config_values<br/>lists and tables in settings"]
    bench_protocol["aspen_bench_protocol<br/>seeding and suspension messages"]

    chat --> app & voice_protocol & limits & metrics & sockets & bench_protocol
    app --> database & voice_protocol & tls & config_values & bench_protocol
    voice_server --> voice_protocol & limits & metrics & sockets & tls & config_values
    migrate --> database & search_text
    bench --> voice_protocol & limits & metrics & bench_protocol
    voice_protocol --> limits
    database --> metrics
```

- `voice_protocol` is the whole contract between the two servers: the join token the API server
  signs and the voice server checks, the reports and commands they exchange over NATS, and the
  signalling frames clients exchange with the voice server.
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
    capture -- "plain RTP" --> voice_server
```

- Plugins and the server share nothing but `spec/plugin.wit`; a plugin is a WebAssembly component
  the operator installs, and `aspen_plugin_runtime` runs it. See
  [Plugins](../architecture/plugins/index.md).
- `aspen_capture` is the desktop app's helper process for game and window capture, built by
  `pnpm build:native` in `client/packages/desktop`, since it links libobs. See
  [Client packages](client.md).
