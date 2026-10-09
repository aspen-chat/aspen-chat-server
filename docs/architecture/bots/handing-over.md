# Handing a bot over

A bot changes hands only with its recipient's consent (`app::bot::offer_transfer`, `accept_transfer`).

## Endpoints

| Endpoint | By | Does |
| --- | --- | --- |
| `PUT /bots/{bot}/transfer` | The owner | Offers the bot to a person of this deployment under the cap. Takes a recently verified sign-in, as issuing a token does |
| `GET /users/@me/bot-transfers` | Anyone | Lists the standing offers made by and to the caller, each with the bot, the giver, and the recipient |
| `DELETE /bots/{bot}/transfer` | The giver or the recipient | Withdraws the offer (giver) or declines it (recipient) |
| `POST /bots/{bot}/transfer/acceptance` | The recipient | Makes them the owner and gives the bot a new token |

## Offering

1. The owner offers the bot. Offering it again replaces the offer: one row of `bot_transfer` per bot.
2. The offer is saved.
3. The recipient is told by a notice from the system account, sent in English. A notice that fails is logged, and the offer stands.

The offer lasts seven days (`TRANSFER_OFFER_DAYS`).

## Accepting

Acceptance makes the recipient the owner. In one transaction:

1. The owner changes, announced as the bot's `user` update.
2. The token is replaced. The new token is answered to the recipient alone, so the old one, which the giver may still hold, stops working.
3. `signInsEnded` closes the bot's event streams and takes it out of its calls, as issuing a token does.

Accepting needs no fresh verification. It takes nothing from the recipient.

## When an offer stands

An offer stands only while it is unexpired and its giver still owns the bot. One whose giver deleted their account (leaving the bot ownerless) or deleted the bot reads as gone.

Acceptance is refused:

- While the bot is shut out of the deployment (`user_ban::shut_out`), which a ban of its giver does.
- When the recipient owns as many bots as they may.

## What the clients show

Nothing about an offer is pushed to either side but the notice.

- The web client reads the offers when Settings' Developer section opens. It lists those made to the person, whether or not developer mode is on.
- The Bots dialog shows each bot's own offer, with a way to withdraw it.

## When access is given or taken away

1. **Who can observe an offer?** Its giver and its recipient, through `GET /users/@me/bot-transfers` alone, and the recipient through the notice.
2. **What decides it?** The row's `from_owner` and `to_user`, checked in the query. At acceptance, that `from_owner` is still the bot's owner.
3. **When it is lost:**
   - When the giver loses the bot (deleting it or their account), the offer stops standing.
   - A ban of the giver refuses acceptance while it lasts.
   - A password reset by email does not touch bot tokens or offers, and neither does a password change. A bot's token is its own credential, which only its owner's fresh verification reissues.
4. **When it is gained:** the recipient's client learns of it from the acceptance's answer and the bot's `user` update. The giver's client sees `botOwner` change on that update and drops the bot from its list.
5. **Does every path announce it?** Offers themselves are never announced: making or withdrawing one publishes nothing, and clients read offers when they show them. Acceptance announces the owner change (the bot's `user` update, `EventScope::UserEverywhere`) and ends the bot's sign-ins (`login::revoke_all_sessions`, which publishes `signInsEnded`). An offer that stops standing because its giver lost the bot is not announced; the reads leave it out.
6. **Is it published inside the transaction?** Yes. `accept_transfer` publishes both events inside the transaction that changes the owner and replaces the token, so a rollback is answered by `app::events::settle`'s resyncs.

`scripts/check_permissions.py` (`bot_transfers`) checks:

- That issuing a token and offering a bot ask for a fresh verification.
- That an offer withdrawn cannot be accepted.
- That acceptance ends the old token and its stream.
