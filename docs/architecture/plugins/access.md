# When access is given or taken away

The revocation checklist (see `AGENTS.md`, Standards and Expectations) answered for plugins.

## Channels, views, notices, cards, and capability URLs

| Feature | Who sees it, and what happens on losing access |
| --- | --- |
| A plugin's channel | It is a channel. Who sees it, its events, and the storage scoped to it is decided by View channel. Losing it ends the plugin's routes' answers about it, as for any channel's storage. |
| A view's files | The same for everyone. They hold nothing of anyone's. |
| A notice | Sent only to someone who may view its channel when it is sent, and read back only while they still may. |
| A card | Part of a message, seen by whoever reads the message. Pressing it takes reading it. |
| A capability URL | Answers as its owner, so it answers nothing once they lose what it shows, are banned, or are deleted. |

`scripts/check_permissions.py` checks each with the calendar.

### Blackjack

A game's table is a plugin's channel too. Blackjack:

- shows its table to whoever may view the channel;
- seats only those who may send messages there;
- keeps each player's chips in their own scope (`wallet:{community}`), which deleting the account deletes;
- answers a route about a table they can no longer view with nothing.

`scripts/check_permissions.py` checks that, and that simultaneous bets keep a table and its players' chips straight.

## 1. Who can observe it, and by which routes?

| What | Who | Routes |
| --- | --- | --- |
| A message's annotations | Whoever may read the message. | Its channel's events, and message reads' sideloads. |
| A person's annotations | Whoever shares a community with them. | Their profile's events, and `GET /users/{user}/annotations`. |
| A community's use of a plugin | Holders of Manage plugins. | `communityPlugin` events, and `GET /communities/{community}/plugins`. |
| A plugin's events | Whoever its audience names. | As any event of that scope. |
| What a plugin keeps | As the caller may look. | Its routes. |
| The catalogue | Every signed-in user. | `GET /plugins`. |

## 2. What decides it, and where is that checked?

| Check | Where |
| --- | --- |
| `channel_access` | `host::Call::readable_message`, `scope`, `annotate_message`, and `read_attachment`. |
| `channel_access` for a route's caller (`caller_views`) | `publish`, `notify`, `send-message`, and `send-card`. |
| `channel_access` for a route's caller (`caller_reads`) | `delete-message` and `add-reaction`. |
| `require_member` | Community scopes, and in a route a community's `publish`, `remove-member`, and `ban-member` (`caller_belongs`). |
| A shared community, or being the person | `annotation::of_user`, for a person's annotations. It answers anyone else an empty list. |
| The event feed's model (`Aspen-Channel`, `Aspen-Requires`) | Annotation, plugin, and `communityPlugin` events. |
| `Permissions::MANAGE_PLUGINS` | `app::plugin::community`. |
| Where the plugin runs | `Plugins::runs_at`. |

## 3. When the deciding permission is lost

| Lost | What happens |
| --- | --- |
| View channel (an override, a role, a move, a removal, a ban) | The channel's annotation and plugin events stop reaching that stream from that point, as for messages. Reads and routes check at each call. |
| Manage plugins | `communityPlugin` events stop at once (the model's `Aspen-Requires`). |
| A shared community (leaving, a removal, a ban) | Their annotations' events stop reaching that stream with the membership's end. `GET /users/{user}/annotations` answers none once no community is shared. |
| The plugin, turned off by a community or by the operator, or removed | Its hooks stop on every server within moments (the cache is forgotten on the announcement). Its routes answer `notFound` once it is off. |
| The plugin, turned off by a community or removed by the operator | Its principal is also taken out, and its role goes with it. |
| The principal's membership or its role's permissions | Its next action is refused by the same checks a member's would be. |
| The principal, banned from the deployment | Every action is refused. |
| `dms` (reinstalling without `--grant-dms`) | Its hooks stop in DMs. |
| A plugin's place in `GET /plugins` | The client drops annotations of plugins `GET /plugins` no longer lists. |

## 4. When it is gained

- A reader who gains View channel reads the channel's messages, and their annotations with them, as any channel gained.
- A member gaining Manage plugins reads `GET /communities/{community}/plugins` when they open the community's plugin settings.
- A plugin turned on appears in `GET /plugins`, which the client reads again when it meets a plugin it does not know.

## 5. Does every path that changes it announce it?

- Community changes publish `communityPlugin` events inside their transaction.
- The principal's joining and leaving publish its membership and role events.
- Annotation changes publish their own events.
- The operator's and the dashboard's changes announce on `aspen.plugins.changed`.
- Removing a plugin deletes its annotations without events. Every client infers that from the catalogue.

## 6. Is it published inside the transaction that makes the change?

Annotation, `communityPlugin`, membership, and role events are. A rollback is answered by `communityResync`, and a membership's by `userResync` for its member too, as for any request.

## What `scripts/check_permissions.py` checks

- A plugin's annotations and events stop reaching someone who loses View channel.
- Its route stops answering them for the channel's storage.
- A community's plugin settings reach only holders of Manage plugins.
- Turning the plugin off takes its principal out.
