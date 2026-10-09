# Event routing

Every event is published on a NATS subject that names whose it is. Each API server reads the stream once and routes each event to the event stream connections allowed to see it. Presence, which is pulled rather than pushed, is described here too.

## Parts

| Part | Where |
| --- | --- |
| Subjects, publishing, settling | `app::events` |
| The event feed: dispatcher, shards, catch-up | `app::event_feed` |
| Who may view a channel or category | `app::visibility` (`CommunityModel`, `Visibility`, `viewers`) |
| Presence | `app::user_status` |
| Online count of a channel | `app::channel_presence` |
| The WebSocket endpoint | `server/api/src/event_stream.rs` |

## Pages

- [Subjects and the feed](subjects-and-feed.md): the subjects, the dispatcher and its shards, what a connection receives, and typing frames.
- [Visibility](visibility.md): the routing headers, the community model, and how each reader's view is decided.
- [Publishing](publishing.md): `publish_event`, `EventScope`, `expected_kind`, and events about a user.
- [Settling and resyncs](settling.md): noting what each request published, rechecking calls, and answering rollbacks.
- [Ending sign-ins](sign-in-ends.md): `signInsEnded`, `accountBanned`, and token expiry on open streams.
- [Catch-up and connection limits](catch-up.md): retained events, resuming, registration, shared frames, stream caps, and dropped connections.
- [Presence](presence.md): online, away, and offline, and how many are online in a channel.

[Design notes](design-notes.md) hold the reasons behind these choices.
