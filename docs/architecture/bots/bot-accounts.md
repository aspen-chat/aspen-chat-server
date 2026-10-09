# Bot accounts

Code: `app::bot`, `api::bot`.

## Signing in

A bot is a user with `bot` set. It signs in only with its token:

- The server keeps the token as a SHA-256 digest (`bot_token`).
- The token begins `aspenbot_`.
- It is sent as the bearer token of every request, and as the `sessionToken` of the event stream's `identify`.

So everything a person may do through the API a bot may do too, under the same permissions and limits.

A bot has no password, second factors, or sessions:

- Password sign-in refuses bots.
- `two_factor::Caller::ensure_person` refuses every security change and reauthentication.

## Joining communities

Bots join communities by accepting invites like anyone, or through their link (see [Adding a bot to a community](adding-to-communities.md)).

## Owners

The person who made a bot owns it (`user.bot_owner`). A bot acts for its owner.

- A person may make at most the deployment setting `bots_max_per_user` bots (see [Administration](../administration/index.md)).
- None may be made while `bots_enabled` is off. That leaves bots already made working.
- While either the bot or its owner is banned from the deployment, its token resolves to no one (`app::user_ban::shut_out`).
- Banning the owner closes the bot's streams and takes it out of its calls (see [Administration](../administration/index.md)).

## Endpoints

| Endpoint | Does |
| --- | --- |
| `GET /users/@me/bots` | Lists the caller's bots |
| `POST /users/@me/bots` | Makes a bot. The answer carries its token, which is never shown again |
| `POST /bots/{bot}/token` | Issues a new token, ending the old. Takes a recently verified sign-in (`reauthenticationRequired`) |
| `PATCH /bots/{bot}` | Makes it public or private |
| `PUT /bots/{bot}/transfer` | Offers it to another person (see [Handing a bot over](handing-over.md)) |
| `DELETE /bots/{bot}` | Deletes it |
| `PATCH /users/{user}` | Changes its profile, by the bot or its owner |

**Why issuing a token needs a fresh verification:** whoever holds a stolen session could otherwise take the bot's token.

## When the owner's account is deleted

Deleting an account (`app::user::retire`) leaves its bots working and ownerless:

- `bot_owner` is cleared, announced as their `user` update.
- A holder of the deployment permission Manage deployment settings may delete them.
- No one may issue them tokens.

## Plugins' bots

A plugin's account, its principal, is a bot no one owns, with `plugin` naming the plugin (records carry it as `plugin`).

- It has no token.
- It acts only for its plugin.
- It goes only with its plugin (`botIsPlugin`; see [Plugins](../plugins/index.md)).

## Records

Records carry `bot`, `botOwner`, and `botPublic`. Clients mark bots wherever they are named.
