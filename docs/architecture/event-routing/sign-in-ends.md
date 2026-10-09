# Ending sign-ins

Each connection knows the sign-in it belongs to (`app::login::sign_in_id`, half the SHA-256 of its refresh token). Events on the user's subject close the streams of sign-ins that end.

## Events

| Event | Published when | Closes |
| --- | --- | --- |
| `signInsEnded` | Signing out; revoking other sign-ins (a password or second factor change); revoking all of them (a ban, an account deleted, a home's standing withdrawn); rotating a bot's token | Only the streams of the sign-ins it names, with 4401, after the event |
| `accountBanned` | An account ban | All the user's streams, with 4410 |

## Sign-ins begun after the end

- Both events carry `at`, the database's clock as they were published (`login::database_clock`). It is after every sign-in they end began.
- A connection knows when its sign-in began: `refresh_token.created_at`, or for a bot its current token's `bot_token.created_at`.
- Neither event reaches nor ends a sign-in begun at or after its `at`. So a new sign-in's first connection, replaying the window, is not closed by an end or a ban from before it existed.

## Ends that race identify

A session is checked against the database when it identifies, possibly before such an end committed.

- A registration whose `resumeAfter` lies at or past a retained end that concerns it is refused with the same code, rather than resuming beyond it (`Retained::catch_up`).
- One whose `resumeAfter` lies before it is replayed the end, and closes after it.

## Expiry

A stream also closes with 4401 when its sign-in's refresh token expires. The moment is fixed when the stream identifies, as the expiry it read then. A bot's token does not expire.
