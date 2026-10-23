# Flows

What happens across the parts of a deployment for three things people do. Each is simplified to the
path that succeeds; the linked pages hold the rest.

## Posting a message

```mermaid
sequenceDiagram
    autonumber
    participant C as Client
    participant A as API server<br/>(api, then app)
    participant P as Plugins<br/>(interceptors)
    participant DB as PostgreSQL
    participant J as NATS JetStream<br/>aspen_omni_stream
    participant F as Every API server's<br/>event feed
    participant Others as Other clients

    C->>A: POST /api/v1/channels/{channel}/messages
    A->>A: rate limits, session, channel_access, mana
    A->>P: intercept (may change or refuse it)
    P-->>A: decision
    A->>DB: BEGIN, then INSERT message
    A->>J: publish on aspen.events.c.{community}.ch.{channel}
    J-->>A: acknowledged
    A->>DB: COMMIT
    A-->>C: 201 Created, the message
    J->>F: one ordered consumer per server
    F->>F: dispatcher, then the shard holding each connection,<br/>which keeps only readers allowed to view the channel
    F->>Others: {"type":"event","sequence":N,...} on /api/v1/events
    Note over J: durable consumers read the same event:<br/>aspen_push wakes phones through their relay,<br/>and each observing plugin is told of it
```

- The event is acknowledged before the transaction commits, so a refused publish rolls the message
  back and PostgreSQL and the stream never disagree on order. A rollback after publishing is
  answered with `communityResync` (see
  [Settling and resyncs](../architecture/event-routing/settling.md)).
- Plugins decide before the transaction opens, so a slow one holds no database connection.
- A message may instead be answered `202 Accepted` and held: in a busy channel, waiting its turn for
  the channel's mana (see [Channel mana](../architecture/channel-mana/index.md)), or while a preview
  of one of its attachments is made (see
  [Attachment previews](../architecture/attachment-previews/index.md)). It is posted later through
  the same transaction and event.

## Joining a call

```mermaid
sequenceDiagram
    autonumber
    participant C as Client
    participant A as API server
    participant K as Valkey
    participant V as Voice server
    participant N as NATS

    Note over V,N: at startup the voice server asks for the join<br/>token key over NATS, and an API server answers
    C->>A: POST /api/v1/channels/{channel}/voice/join
    A->>A: permissions, choose candidates
    A->>K: note the servers offered
    A-->>C: join token (Ed25519), candidates, grants
    C->>V: GET /health on each candidate, nearest first
    C->>V: WebSocket GET /ws: identify with the token
    V->>V: verify the token, use it once
    V-->>C: ready: RTP capabilities, who is in the call
    C->>V: setCapabilities, transports, produce microphone
    C-)V: media (DTLS-SRTP)
    V-)N: report on aspen.voice.report.{lane}.{server}
    N-)A: one durable consumer per lane, shared by every API server
    A->>A: apply the report: voice session rows,<br/>events to the channel's viewers
    Note over A,V: when access changes, the API server commands<br/>the voice server on aspen.voice.command.{server}
```

- The voice server never sees the database or a session token; the join token and the grants in it
  are all it knows of the joiner, and rechecks keep the grants current. See
  [Join tokens](../architecture/voice/join-tokens.md) and
  [Rechecks and grants](../architecture/voice/rechecks.md).
- A report is believed only from the server its subject names, and only for a call or participant
  that server was offered for (the note in Valkey). See
  [Voice reports](../architecture/voice/voice-reports.md).

## Signing in to another deployment

```mermaid
sequenceDiagram
    autonumber
    participant C as Client
    participant H as Home deployment
    participant X as Other deployment
    participant K as Other's Valkey

    C->>H: POST /api/v1/auth/assertions {audience}
    H->>H: emigration gate for the audience
    H-->>C: assertion: compact JWS (EdDSA), two minutes of life
    C->>X: POST /api/v1/auth/federated-sign-in
    X->>X: immigration gate for the issuer
    opt key not yet pinned, or the signature fails once
        X->>H: GET /.well-known/aspen (identity and key)
        H-->>X: document
    end
    X->>X: verify signature, audience, lifetime, gate, sign-in strength
    X->>K: remember the assertion's id until it expires
    X->>X: make or update the foreign user
    X-->>C: an ordinary session on the other deployment
```

- The client then holds a session on each deployment and talks to each directly; the deployments
  talk to each other only to verify and to send notices (DMs across deployments, standing checks,
  deletions). See [Signing in abroad](../architecture/federation/signing-in-abroad.md) and
  [Federation](../architecture/federation/index.md).
