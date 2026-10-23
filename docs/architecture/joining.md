# Default channels and join messages

Each community has a default channel: the text channel new members are shown first, and where a
message announces each one who joins.

| Part         | Where                                                                                                                                                   |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Logic        | `app::default_channel`                                                                                                                                  |
| Columns      | `community.default_channel` (a channel, `NULL` while the community has no text channel) and `community.suppress_join_messages`                          |
| Wire         | `Community.defaultChannel` and `Community.suppressJoinMessages`, changed by `PATCH /communities/{community}` and announced as the community's `update`  |
| Message kind | `memberJoined` (`member_joined`), its `author` the one who joined, with no content                                                                      |
| Client       | `landingChannel` (`client/packages/app/src/features/channels/landing.ts`), `JoinNotice`, and the New members section of a community's Overview settings |

## The default channel

- A new community's first text channel is its default, and so is the first text channel made in a
  community that has none (`channel_created`).
- A holder of Manage community may name another: one of the community's live text channels that they
  may view. Naming a channel they may not view is answered as not found, another kind of channel is
  refused with `validation`, and so is `null`, since a community with a text channel always has a
  default. A create request may not name one.
- When the default channel is deleted, `choose` picks the next, inside the deleting transaction, and
  announces it. A community left without a text channel has none.
- Each change holds the community's row (`hold_community`), so two channels deleted at once cannot
  each pick the other, and a channel being named holds the channel's row, so it cannot be deleted
  before the community names it.

### Choosing the next one

`choose` takes the oldest of the community's live text channels that everyone's role may view, after
its category's override and then its own for everyone's role (`CommunityModel::everyone_views`), or
the oldest of all when everyone's role may view none. A channel's age is its UUIDv7 id, so the order
needs no other column.

The `join_messages` migration gave each existing community the channel `choose` would pick.

## Join messages

`announce_join` posts one, in the joining transaction, when someone joins by an invite
(`app::community::join_community`), when registering with a dual invite joins them
(`app::registration_invite`), and when a bot is added (`app::bot`). No message is posted for a
community's creator, for a plugin's account (`app::plugin::principal`), when the community has no
default channel, or when `suppress_join_messages` is set; the client's checkbox is "Don't post a
message when someone joins".

A join message:

- is posted in the name of the one who joined, and is read by them already (`read_state::advance`);
- counts toward the channel's unread state as any message does;
- notifies no one, wakes no phone (`app::push`), and is left out of search and the activity feed, as
  a poll's result and a call's record are;
- cannot be edited (`app::message::update_message`); the client offers no thread or report on one,
  as on a call's record.

Someone who leaves and joins again is announced again. A ban that deletes the member's recent
messages deletes their join messages among them (see [Bans](roles-and-permissions/bans.md)).

## Who may see what

1. **Observing.** A join message is a message of its channel: its readers alone receive it, in REST
   reads, on the event stream (`EventScope::Channel`), and wherever its channel's messages are read.
   The default channel's id, and whether join messages are posted, are part of the community's
   record, which every member reads and receives (`EventScope::Community`). A member who may not
   view the default channel learns only its id, never its name, and the client opens it only when
   the member has it (`landingChannel`), otherwise the first text channel they have.
2. **Deciding.** Reading a join message takes viewing its channel (`channel_access`). Changing the
   default channel takes Manage community and viewing the channel named
   (`app::community::update_community`); changing `suppressJoinMessages` takes Manage community. A
   deployment moderator may rename a community and change neither.
3. **Losing access.** Losing the view of the default channel is losing the view of any channel:
   later join messages stop reaching the member, and their client opens their first text channel
   instead. Losing Manage community hides the New members section, and the server refuses the
   change. Neither the default nor whether joins are posted gives anyone anything, so nothing
   already open needs rechecking.
4. **Gaining access.** Gaining the view of the default channel is gaining a channel. A changed
   default reaches every member as the community's `update`.
5. **Every path announces.** A change by request, a channel made in a community without a default,
   and a deleted default are each announced. The migration and `bench seed`, which run with no one
   connected, announce nothing.
6. **Inside the transaction.** Every change and every join message is published inside the
   transaction that makes it, so a rollback is answered by `communityResync`
   (`app::events::settle`).

`scripts/check_permissions.py`'s `join_messages` scenario checks these.
