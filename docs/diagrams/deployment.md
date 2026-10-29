# The deployment

A deployment is one origin, `public_url`, served by any number of API servers, plus any number of
voice servers, sharing PostgreSQL, NATS JetStream, Valkey, and S3-compatible storage. The smallest
runs all of it on one machine (`deploy/box/`); a large one runs each part on machines of its own
(`deploy/aws/`). The shape is the same.

## Processes and services

```mermaid
flowchart TB
    subgraph clients["Clients"]
        browser["Web browser<br/>(the web client)"]
        desktop["Desktop app<br/>(Electron + aspen-capture)"]
        phone["iOS and Android apps<br/>(Capacitor)"]
    end

    proxy["TLS terminator or load balancer<br/>(Caddy in aspen-box; optional)"]

    subgraph api_tier["API servers (aspen-chat-server), any number"]
        api["API server<br/>REST /api/v1, event stream /api/v1/events,<br/>web client, /.well-known/aspen"]
        worker["Private worker<br/>(--private-worker: background work only)"]
    end

    subgraph voice_tier["Voice servers (voice_server), any number"]
        voice["Voice server<br/>mediasoup SFU, signalling GET /ws,<br/>STUN and TURN for file transfers"]
    end

    subgraph shared["Shared services"]
        pg[("PostgreSQL<br/>every record")]
        nats[("NATS JetStream<br/>events, voice reports and commands,<br/>settings, fleet heartbeats")]
        valkey[("Valkey<br/>rate limits, presence, ceremonies,<br/>voice offers, single-use ids")]
        s3[("S3-compatible storage<br/>attachments, icons, previews<br/>(Garage in aspen-box)")]
    end

    subgraph outside["Outside the deployment"]
        smtp["SMTP server"]
        relay["Push relay (aspen-push-relay)<br/>or UnifiedPush distributor"]
        others["Other Aspen deployments"]
        web["Linked web pages<br/>(link previews)"]
    end

    clients -- "HTTPS, WebSockets" --> proxy
    proxy --> api
    proxy -- "voice signalling" --> voice
    clients -- "media (UDP), TURN" --> voice
    clients -- "presigned uploads, downloads" --> s3

    api_tier --> pg & valkey & s3
    api_tier <--> nats
    voice <-- "reports, commands, token keys" --> nats

    api_tier -- "mail and digests" --> smtp
    api_tier -- "Web Push" --> relay
    api <-- "signed statements" --> others
    api -- "fetch" --> web
```

- Every API server serves the same origin; any of them may answer any request, and each holds its
  own event stream connections. A private worker takes no requests and only shares the background
  work (mail, voice reports, standing checks, plugins' observers and timers, push). See
  [the web client](../architecture/web-client/index.md) and `app::context::Role`.
- Clients signal with a voice server over its own WebSocket (`GET /ws`, through the proxy where one
  terminates TLS) and send media straight to it. The API server offers them candidates and signs the
  token they join with; the voice server never reads the database. See
  [Voice](../architecture/voice/index.md).
- Clients upload attachments straight to storage with presigned URLs, and download them from
  `public_base_url`. The API server only signs, confirms, and makes previews (with `ffmpeg` and
  `ffprobe` as processes of their own for videos). See
  [Attachment previews](../architecture/attachment-previews/index.md).
- Phones are woken through their app's relay, which alone holds the Apple and Google credentials.
  See [Push](../architecture/push.md).
- Requests to other hosts (other deployments, link previews, push relays) go through
  `aspen_outbound`, which refuses private addresses unless the development settings allow them.

## What NATS carries

```mermaid
flowchart LR
    subgraph writers["Writers"]
        pub["API servers: publish_event,<br/>inside the SQL transaction"]
        typer["API servers: typing frames"]
        v_out["Voice servers"]
        cmd_out["API servers: rechecks,<br/>kicks, mutes"]
        settings_out["API servers: settings changes,<br/>fleet heartbeats"]
    end

    subgraph js["NATS"]
        omni[["stream aspen_omni_stream<br/>aspen.events.c.{community}.ch.{channel}<br/>aspen.events.c.{community}.all<br/>aspen.events.u.{user}<br/>aspen.events.p.{user}<br/>in memory, one minute"]]
        typing(["aspen.typing<br/>core NATS, never kept"])
        vrep[["stream aspen_voice_reports<br/>aspen.voice.report.{lane}.{server}<br/>aspen.voice.speaking.{lane}.{server}<br/>in memory, work queue"]]
        tok(["token key requests<br/>core NATS"])
        cmd(["aspen.voice.command.{server}<br/>core NATS"])
        kv[("key-value buckets<br/>aspen_settings, aspen_fleet")]
    end

    subgraph readers["Readers"]
        feed["Each API server's event feed:<br/>one ordered consumer, then shards"]
        pushd["Push dispatcher:<br/>durable consumer aspen_push"]
        obs["Plugin observers:<br/>a durable consumer each"]
        rep["Report readers: a durable<br/>consumer per lane, shared"]
        ans["Token key answerer:<br/>one queue group"]
        v_in["Voice servers"]
        copies["Every server's settings copy,<br/>the dashboard's fleet section"]
    end

    pub --> omni
    typer --> typing
    v_out --> vrep & tok
    cmd_out --> cmd
    settings_out --> kv

    omni --> feed & pushd & obs
    typing --> feed
    vrep --> rep
    tok --> ans
    cmd --> v_in
    kv --> copies
```

- Every change is published to `aspen_omni_stream` before its transaction commits, and committed
  only once JetStream acknowledges it, so the database and the stream agree on order. See Event
  Ordering Guarantee in `AGENTS.md` and [Event routing](../architecture/event-routing/index.md).
- Each API server reads the whole stream once and routes each event to the connections allowed to
  see it, through `event_feed_shards` routing tasks. A reconnecting client resumes from its last
  sequence while that is within the minute the stream keeps.
- Voice servers publish only on their own report subjects and are commanded on their own command
  subject; the stream is shared by every API server, one durable consumer per lane. See
  [Voice reports](../architecture/voice/voice-reports.md).
- `aspen_settings` holds the deployment settings each server keeps a copy of, and `aspen_fleet` each
  API server's heartbeat, which the dashboard's fleet section reads. How large the two in-memory
  streams may grow is in [NATS memory](../architecture/nats-memory.md).

## One machine and many

```mermaid
flowchart TB
    subgraph box["aspen-box: one Linux machine, Docker Compose"]
        caddy["Caddy<br/>public_url, voice., media."]
        b_api["aspen-api container"]
        b_voice["aspen-voice container"]
        b_pg[("PostgreSQL<br/>(or elsewhere)")]
        b_nats[("NATS")]
        b_valkey[("Valkey")]
        b_garage[("Garage<br/>(or elsewhere)")]
        caddy --> b_api
        caddy --> b_voice
        caddy -- "media.: presigned uploads<br/>and the read path" --> b_garage
        b_api --> b_pg & b_nats & b_valkey & b_garage
        b_voice --> b_nats
    end

    subgraph more["More voice servers (init-voice)"]
        v2["aspen-voice"]
    end
    v2 -- "NATS over TLS, from the box's own authority" --> b_nats

    subgraph aws["deploy/aws: the larger configuration"]
        lb["Load balancer<br/>(API and voice signalling)"]
        a_api["API servers"]
        a_voice["A machine per voice server"]
        a_pg[("RDS PostgreSQL")]
        a_rest[("NATS task, ElastiCache Valkey, S3")]
        lb --> a_api & a_voice
        a_api --> a_pg & a_rest
        a_voice --> a_rest
    end

    box -. "moves to" .-> aws
```

Both run the images `.github/workflows/images.yml` publishes (`ghcr.io/aspen-chat/aspen-api` and
`aspen-voice`) and write their configuration from `deploy/templates/`. A box that outgrows its
machine moves to the larger configuration with `deploy/aws/migrate.py`, which restores its database
from a dump and copies its files or hands over its bucket. See
[`deploy/README.md`](../../deploy/README.md).
