# Voice

Calls run on voice servers: separate mediasoup processes that the API servers register, offer to joiners, command over NATS, and keep a record of through the reports the voice servers send. A client never authenticates with a voice server directly; it joins with a token the API server signs.

## Pages

- [Voice servers and the registry](servers.md): the `voice_server` table, the `/voice-servers` endpoints, and the `voice-servers` operator command.
- [Join tokens](join-tokens.md): Join voice, the token's claims and grants, the Ed25519 key, and one connection per token.
- [Choosing a server](server-selection.md): candidates, failure reports, and suspension.
- [Rechecks and grants](rechecks.md): keeping calls in line when access changes, kicks, ended sign-ins, and grants.
- [Voice reports](voice-reports.md): the `aspen_voice_reports` stream, its lanes, where a report came from, NATS users, and retries.
- [Snapshots](snapshots.md): how a voice server's snapshot repairs lost reports.
- [Sessions and their ending](sessions.md): applying reports, silence, the reaper, duplicate rooms, ending reasons, and rejoining.
- [Moderation and server mutes](moderation.md): voice commands, server-muting and removing participants, and the community's standing mutes.
- [The voice server process](voice-server.md): configuration, the announced address, connection and rate limits, seats and transports, and capacity estimates.
- [Signalling and media](signalling.md): the signalling protocol, plain RTP producers and consumers, codecs, mute and deafen, screens and cameras, and rooms.
- [Design notes](design-notes.md): why the pieces are shaped as they are.

## Parts

| Part | Where it lives |
|---|---|
| Shared protocol: join token, control messages, signalling frames | `voice_protocol/` (`voice_protocol::token`, `voice_protocol::control`, `voice_protocol::signal`) |
| API side: offers, rechecks, reports, reaper, mutes | `app::voice` (`app::voice::reports`, `app::voice::sessions`, `app::voice::mutes`, `app::voice::recheck`) |
| Join token signing key | `app::server_secret::JoinTokenKey` |
| Media process | `voice_server/` |
| Shared limit format and GCRA | `limits/` (`aspen_limits`) |
| Client | `VoiceCall` in `client/packages/protocol/src/voice.ts` (see [client voice](../../../client/docs/architecture/voice/index.md)) |
| Hand-driven test server | `voice_protocol/examples/fake_voice_server.rs` |

The report listener (`app::voice::spawn_report_listener`) is started with the other background tasks (see [Background tasks](../background-tasks.md)). The reaper is the recurring `reapVoice` job (`app::voice::reap`; see [Jobs](../jobs/index.md)). File transfers in calls are in [File transfers](../file-transfers.md).
