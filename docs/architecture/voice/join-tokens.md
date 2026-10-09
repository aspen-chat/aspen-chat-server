# Join tokens

A client never authenticates with a voice server directly. It asks the API server for a join token and presents it to the voice server in its first signalling frame (`identify`; see [Signalling](signalling.md)).

## Where it lives

| Piece | Code |
|---|---|
| Signing | `voice_protocol::token::sign` |
| Shared-secret tokens | `voice_protocol::token::verify_shared` |
| Grants | `voice_protocol::token::Grants` |
| Claims | `JoinClaims` (`JoinClaims::server_muted`) |
| Signing key | `app::server_secret::JoinTokenKey`, kept in `server_secret` |
| Key answerer | `app::voice::spawn_token_key_answerer` |
| Key fetching on the voice server | `voice_server/src/token_keys.rs` |
| Used tokens | `UsedTokens` |

## Join voice

`POST /channels/{channel}/voice/join` (Join voice) returns:

- a token naming:
  - the user,
  - the sign-in it was issued to (`login::sign_in_id`; none for a bot's token),
  - the channel,
  - the candidate servers,
  - the grants,
  - whether a server mute stands (`JoinClaims::server_muted`; see [Moderation](moderation.md#server-mutes));
- those candidates and the grants;
- whether a server mute stands (`serverMuted`).

A call is held in a voice channel, a DM, or a group DM (see [Threads and DMs](../threads-and-dms/index.md)). Any other channel is refused with `voiceChannelOnly`. How candidates are chosen is in [Choosing a server](server-selection.md).

### Grants

The grants are the joiner's permissions as they stood when the token was issued:

| Grant | Permission | Allows |
|---|---|---|
| `speak` | Speak | Microphone producer |
| `share_screen` | Share screen | Screen and screen audio producers |
| `camera` | Use camera | Camera producer |
| offer files | Transfer files | File offers (see [File transfers](../file-transfers.md)) |

The voice server holds the participant to the grants as they now stand, which [rechecks](rechecks.md) keep current.

## The signing key

1. The first API server to start makes an Ed25519 key (`JoinTokenKey`) and keeps it in `server_secret`.
2. API servers sign tokens with it. A token is the key's id, the claims, and the signature.
3. A voice server holds only the public half. It can check tokens but never make one.

### How a voice server gets the public half

- It asks over NATS on `control::TOKEN_KEY_SUBJECT`. Every API server answers, in one queue group.
- It asks at startup, retrying until one answers.
- It asks again when a token names a key it does not hold, at most once every five seconds.
- Only the API servers may publish to a voice server's inbox, so the answer is theirs (see [NATS users](voice-reports.md#nats-users)).
- The answer goes only to a subject in the asking server's inbox (`voice_protocol::control::is_voice_inbox`). A request naming any other reply subject is logged as a sign of a compromised voice server and left unanswered.

### Checking the voice server's id

A voice server learns whether its id is registered when it starts (`token_keys`):

1. Its request for the key is a `TokenKeyRequest` naming its id.
2. The API server answering looks the id up and says in the `TokenKey`'s `registered`.
3. A voice server given the wrong id stops at startup with an error naming it, rather than reporting to no one.

Other answers:

- An empty request is answered with the key alone.
- An answer without `registered` is taken as no answer to that question, and the server goes on. That happens on a database error, or when the API server is already running `REGISTRATION_LOOKUPS` (two) of these look-ups. A voice server asking over and over then holds at most that many database connections.

The id is only asked about, never believed: it decides nothing but what the voice server is told.

### Shared-secret tokens

A voice server given `token_secret` also takes tokens of the shared-secret form under it (`token::verify_shared`). This is how a deployment upgrades from API servers that made those (see [Upgrading](../../operators/installing/upgrading.md#from-shared-secret-join-tokens)). The voice server refuses to start with a `token_secret` that is the development value or shorter than 32 bytes, unless `development` is set.

## One connection per token

A token admits one connection to each server it names.

- The voice server remembers the nonces of tokens it has accepted until they expire (`UsedTokens`).
- A token used again is refused.

**Why:** someone removed cannot come back on the token they joined with.

A token made while the channel had no call names up to ten servers, so it can open rooms on two of them; see [Duplicate rooms](sessions.md#duplicate-rooms).
