# Following threads

Code: `app::thread_follow`. Table `thread_follow` holds the user, the thread (each cascading), and when they last followed it.

## What a follow does

Every reply in a followed thread tells its follower:

- By push.
- By the apps' notifications.
- In the activity feed (see [Activity feed](../activity-feed.md)).

This holds whatever their notification level for the parent. A mute of the parent still silences it.

## How a user comes to follow

| Way | Detail |
| --- | --- |
| By hand | `PUT /channels/{channel}/follows/@me`: `201` or `200` |
| Starting it | The author of the message a thread starts from follows it as it is made |
| Posting in it | A poll included. Follows again and marks it followed now |
| Being tagged by name in a reply while able to read it | Follows again and marks it followed now |

`DELETE /channels/{channel}/follows/@me` stops following. That lasts until they follow again or take part again.

Bots, the system account, and deleted accounts follow only by hand, if at all.

A follow of a DM's thread tells only the DM's people, whoever followed it before leaving.

## The cap

A user follows at most `MAX_THREAD_FOLLOWS` (1000) threads on a deployment. Past that, those followed least recently are let go, each announced.

## Listing and events

- `GET /users/@me/thread-follows` lists the threads they follow that they may read now.
- Each change is published to their own subject as `threadFollowChanged`.
- Following and unfollowing are published inside their transactions. A rollback is answered by `userResync`.

## When the user loses the parent

A follow of a thread whose parent the user loses stays, and tells them nothing while they may not read it:

- Push asks who may view the parent.
- The feed reads only what they may read.
- The event stream carries none of it.
- The list leaves it out until they may read it again.
