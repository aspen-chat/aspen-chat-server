# Pins

A pin is a row of `pin`: the message, its channel, when it was pinned, and its place among the
channel's pins (`sort_index`, each new pin after every one already there).

| Part       | Where                                                         |
| ---------- | ------------------------------------------------------------- |
| Logic      | `app::message::set_pinned`, `app::channel::read_channel_pins` |
| Limit      | `app::message::MAX_PINS` pins a channel                       |
| Permission | `pinMessages` in a community; any recipient in a DM           |

## Endpoints

| Endpoint                         | Does                                                              |
| -------------------------------- | ----------------------------------------------------------------- |
| `PUT /messages/{message}/pin`    | Pins it, after every pin already there; `200` when already pinned |
| `DELETE /messages/{message}/pin` | Unpins it                                                         |
| `GET /channels/{channel}/pins`   | Lists the channel's pins in order                                 |

Pinning and unpinning take the channel row's lock, so two pins cannot take one place and the limit
holds under concurrent pins. Each is announced in the channel as the `pin` `create` or `delete`
event, carrying the message id.

## When a pinned message is deleted

Deleting a message (`app::message::soft_delete`, `soft_delete_many`) deletes its pin in the same
transaction and announces the `pin` `delete` before the message's own, so no client ever holds a pin
of a message it has dropped. Every path that deletes a message goes through those two: its author's
or a moderator's deletion, the ban deletion job, report actions, and a community's takedown.

**Why:** a deleted message is left out of `GET /channels/{channel}/pins` and refused by
`set_pinned`, so a pin left standing could never be taken down and would count toward `MAX_PINS` for
good.

## Who may see a pin

- A channel's pins are read through `channel_access_reading`, like its messages.
- The `pin` events are in the channel scope, so they reach whoever may view the channel at that
  moment and nobody after they lose it.
- A client losing a channel drops its pins with it.
