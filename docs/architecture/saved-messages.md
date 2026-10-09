# Saved messages

A user may save messages for themself, to come back to: a list of their own that no one else sees.

| Part | Where |
| --- | --- |
| Logic | `app::saved_message` |
| Table | `saved_message` (below) |
| Event | `savedMessageChanged`, on the user's own subject |
| Readability check | `app::search::readable_everywhere` |
| Limit | `MAX_SAVED_MESSAGES` (1000) per user per deployment |
| Rate limits | Saving and unsaving have their own |

## The table

`saved_message` holds:

- an id of its own: a UUIDv7 of the saving, which orders the list newest first and pages it;
- the user;
- the message.

The user and message each cascade, and the pair is unique.

## Across deployments

- A save is kept on the deployment that holds the message, under the session the user holds there, a foreign user's included.
- Apps signed in to several deployments merge each one's list, so saving needs no federation of its own.

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /users/@me/saved-messages/{message}` | Saves a message (`201`, or `200` with the save it has) |
| `DELETE /users/@me/saved-messages/{message}` | Removes the save |
| `GET /users/@me/saved-messages` | Lists every save as `{id, message}` |
| `GET /users/@me/saved-messages/messages` | Reads the messages themselves, newest save first |

The messages read takes `limit` (50, at most 100), pages after the save of `before` (a message), and takes the same `include` names as any message read.

## The limit

- A user keeps at most `MAX_SAVED_MESSAGES` (1000) on a deployment.
- One more is refused with `validation`, saying to remove some.
- Saves are made one at a time per user, under a lock on their `user` row, so the limit holds however many arrive at once.

## Events

Every change is published to the user's own subject as `savedMessageChanged`, so their other devices follow. It carries the message, and the save's id while it is saved (`null` once not).

## When access is given or taken away

1. **Who can observe it, and by which routes?** Only the saver. The three reads above answer for the caller alone, and `savedMessageChanged` goes to their user subject. The message itself is read as any message is; a save adds no route to it.
2. **What decides it, and where is that checked?** Saving takes `channel_access` on the message's channel, so a deployment moderator saves no DM they are not in. Reads keep only saves whose message, and its channel, are live and that the user may read now (`app::search::readable_everywhere`: the channels they may view in their communities, the DMs they are in, and both's threads).
3. **When the deciding permission is lost?** The save stays, unlisted.
   - Every way of losing a channel (a role's permissions, a role taken or deleted, an override, a move, a category deleted, a removal, a ban, leaving, a DM left) makes the reads leave it out. It shows again if the channel comes back.
   - Signing out, a password change, or a ban from the deployment end the session, and with it every read.
   - An account deleted takes its saves with it.
   - The client's cache keeps only ids. A message whose channel the store drops is read on demand and, refused, marked missing, which takes its row away.
4. **When it is gained?** The reads list it again. An open Saved page lists it once the user next opens it, the save ids having stayed in the store.
5. **Does every path that changes it announce it?**
   - Saving and unsaving announce it.
   - Deleting the message does: `app::message::soft_delete` deletes its saves and tells each saver, even one who may no longer read the channel and so hears nothing else of the deletion.
   - A message removed outright (a purge, a channel or account deleted) goes with its saves by cascade, and readers infer it from the message's going.
6. **Is it published inside the transaction that makes the change?** Yes. A rollback is answered by `userResync`.
