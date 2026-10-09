# Bots: design notes

Why bots work as they do. The how is in the [main pages](index.md).

## Bot accounts

See [Bot accounts](bot-accounts.md).

- **A bot uses its token exactly as a person uses a session token.** So everything a person may do through the API a bot may do too, under the same permissions and limits.
- **Tokens are kept as SHA-256 digests and shown once.** As refresh and session tokens are (see [Sessions](../sign-in-and-security/sessions.md#tokens)).
- **Turning `bots_enabled` off stops new bots but leaves existing ones working.**
- **A ban of the owner shuts the bot out too.** A bot acts for its owner.
- **Issuing a token needs a fresh verification.** Whoever holds a stolen session could otherwise take the bot's token.
- **An ownerless bot keeps working, but no one may issue it a token.** Holders of Manage deployment settings may delete it.

## Handing a bot over

See [Handing a bot over](handing-over.md).

- **A bot changes hands only with its recipient's consent.**
- **Offering needs a fresh verification; accepting does not.** Accepting gives the recipient the bot, so the offer is the security change. Accepting takes nothing from the recipient.
- **Acceptance issues a new token answered to the recipient alone.** The giver may still hold the old one, which then stops working.
- **The notice is sent after the offer is saved, and its failure does not undo the offer.**
- **Password changes and resets do not touch bot tokens or offers.** A bot's token is its own credential, which only its owner's fresh verification reissues.
- **Nothing about an offer is pushed but the notice.** Clients read offers when the relevant settings open.

## Adding a bot to a community

See [Adding a bot to a community](adding-to-communities.md).

- **Permissions given to a bot go on a role of its own, holding only permissions the caller holds.** The role cannot be given to anyone else or deleted, and goes when the bot leaves.

## Commands

See [Commands](commands.md).

- **One list per bot, the same everywhere.** A bot of another deployment publishes its list on each deployment it uses.
- **Patterns are limited to a subset every client reads alike.** So a pattern matches the same values in every client and on the server.
- **An invocation is checked in the transaction that makes its message.**
- **A command message tags no one, fetches no link preview, wakes no phone, and cannot be edited.** Only the bot hears of it, as `botCommandInvoked`.
