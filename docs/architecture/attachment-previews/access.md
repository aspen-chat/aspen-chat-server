# Access

## When access is given or taken away

### 1. Who can observe it, and by which routes?

| What | Who, and by which routes |
| --- | --- |
| A preview | Whoever may read its attachment (`app::attachment::read_attachment`): its uploader, or whoever may view a channel holding it in a message not deleted. By REST reads of the attachment and its sideloads, and `attachmentPreviewed` on the subject of each message's channel (with `Aspen-Channel`) or the uploader's own. It is served on the same anonymous-read path as its original, under a key naming the same unguessable id. |
| A held message | Its author alone: the `202`, `GET /users/@me/held-messages`, and `heldMessagePosted` and `heldMessageFailed` on their own subject. |

### 2. What decides it, and where is that checked?

- A preview follows its attachment. The event's scope is chosen in `preview::announce`:
  - `Message` for each message (`EventScope::Message`, routed by the channel's view permission); or
  - `User` for the uploader.
- `expected_kind` pairs `attachmentPreviewed` with a channel scope when it names a message, and the user's otherwise.
- A held message is checked as a message is when it is held, and again when it is posted, by `check_posting` in the posting's transaction.

### 3. When the deciding permission is lost, what happens to what is already open?

| Change | Effect |
| --- | --- |
| A reader loses view of the channel | They lose the attachment as they lose the message, and receive no `attachmentPreviewed` after. |
| A preview URL already read | Stays fetchable, as the original's does, since both are on the anonymous-read path. When the message is deleted or the attachment taken off it, both move off that path within about five seconds (`app::attachment::evidence`). |
| A held message's author loses the right to post, or is removed, banned, or blocked by the DM's other person | Dropped when it would be posted. Only its author hears of it. |
| A sign-out or a password change | Does not drop a held message. It was sent while signed in, as a message posted at once would have been. |
| Account deleted | Its held messages are dropped when they would be posted. |

### 4. When it is gained, how does a client already open find out?

- A preview is gained only by being made, which `attachmentPreviewed` announces.
- A reader newly let into a channel reads the attachments they lack, previews included.

### 5. Does every path that changes it announce it?

- Making a preview announces it in the transaction that records it.
- Deleting an attachment, by its uploader or by the sweep of unsent ones, is possible only while it is in no message. So nobody but its uploader held it, and nothing is announced.
- Posting and dropping a held message announce it in their transactions.
- A held message of a deleted account or channel is dropped, and announced, when it would be posted.

### 6. Is it published inside the transaction that makes the change?

Yes. A posting that rolls back publishes `communityResync` for its community, as any request's does (`app::events::settle` runs after each release).

## Checks

`scripts/check_permissions.py` checks that:

- a preview made in a channel a member may not view reaches them neither by event nor by REST;
- a held message reaches nobody but its author until it is posted;
- a held message whose author loses the right to post is dropped with its reason, and is never seen;
- a dropped first reply takes the thread it made with it, while a dropped reply leaves a thread that has another.
