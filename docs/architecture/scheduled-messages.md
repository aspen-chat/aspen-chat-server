# Scheduled messages

Someone can write a message now and have it posted later (`app::message::scheduled`). Until it is
posted, only its author is told of it.

A scheduled message is a held message ([Held messages](attachment-previews/held-messages.md)) that
waits for a time rather than for a preview: it is checked, kept, posted, and dropped by the same
code, and this page says what is its own. [Why](scheduled-messages-design-notes.md)

## What can be scheduled

A message of text, files, or both, with what it replies to, in a channel, a thread already made, a
DM, or a group DM. Not a poll, a slash command, the reply that makes a thread, or a reply's echo to
its thread's parent. A bot schedules nothing (`403` `forbidden`): it keeps time of its own.

| Bound                         | Value                            | Past it            |
| ----------------------------- | -------------------------------- | ------------------ |
| The soonest it may be due     | A minute from now (`MIN_AHEAD`)  | `400` `validation` |
| The furthest                  | Two years from now (`MAX_AHEAD`) | `400` `validation` |
| One author's, failed ones too | A hundred (`MAX_PER_AUTHOR`)     | `400` `validation` |

The count is checked holding the author's own row, so two requests at once cannot both find room.

## The row

`held_message`, with `waiting` `scheduled` and these columns of its own:

| Column        | Holds                                                                                                                    |
| ------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `send_at`     | When it is due. It stays set once the row waits for mana, which marks the row as a scheduled one                         |
| `failure`     | Why it could not be posted, in the language it was scheduled in; set, nothing posts it                                   |
| `failed_at`   | When it failed, which the sweep of failed ones goes by                                                                   |
| `stuck_since` | When it first could not go forward for a reason that may pass: due with no room in the queue, or paid for but not posted |

Its author's apps hold it as a `ScheduledMessage`, whose `status` is read from the row:

| `status`         | The row                                                      |
| ---------------- | ------------------------------------------------------------ |
| `scheduled`      | `waiting = 'scheduled'`, no `failure`                        |
| `waitingForMana` | `waiting = 'mana'`, or due (`send_at` passed) and not failed |
| `failed`         | `failure` set                                                |

## Endpoints

| Endpoint                                              | Does                                                                                  |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `POST /channels/{channel}/scheduled-messages`         | Schedules one for `sendAt`                                                            |
| `GET /users/@me/scheduled-messages`                   | Every one the caller has, the soonest due first: at most a hundred, so all of them    |
| `GET /users/@me/scheduled-messages/{scheduled}`       | One, and with `include=attachments` its files' records                                |
| `PATCH /users/@me/scheduled-messages/{scheduled}`     | Gives it another `sendAt`; a failed one is tried again then                           |
| `POST /users/@me/scheduled-messages/{scheduled}/send` | Makes it due now, a failed one too; it is posted moments later, as it would have been |
| `DELETE /users/@me/scheduled-messages/{scheduled}`    | Deletes it                                                                            |

Scheduling checks what posting now would (`message::check_posting`, `reply::check`): the right to
send there, each file being the author's own and ready, and what it replies to being a live message
of the channel. Nothing is spent of the channel's mana, and no plugin sees it, until it is posted.

`GET /users/@me/held-messages` leaves scheduled messages out: its rows are what an app shows waiting
above the message box, moments from being posted.

## When it is due

Each has a `releaseHeldMessage` job keyed by it, due at `send_at` (see
[Jobs](jobs/kinds.md#releaseheldmessage)). When the job runs (`held::release_step`):

1. An earlier one of the same author in the same channel that is still to be posted is waited for, a
   second at a time (`scheduled::behind_another`), so several due together post in the order they
   are due, then the order they were scheduled. One that failed, waits for mana, or is ten minutes
   past its time is not waited for.
2. It asks the channel's mana, as posting does. When the channel is busy it joins the queue (below).
3. It is posted as it was scheduled (`message::post`), in the language it was scheduled in, with the
   author's permissions as they are then and the channel's plugins deciding it then. What it replies
   to may have been deleted since; it is posted replying to it all the same, tagging no one.
4. The row goes in the posting's transaction, and `heldMessagePosted` names the message it became.

### Another time, or now

`PATCH` and `send` set `send_at`, clear a failure, and give the row a new job in place of whichever
it had (`scheduled::set_time`), in one transaction. A row waiting for mana leaves the queue when it
is given another time (`waiting` back to `scheduled`); sending it now changes nothing, since it is
due already.

A step of the replaced job may be running, or the drainer may be about to post the row it read from
the queue. Whatever either does to a row, it does only while the row still stands as it read it:
posting takes the row only while it is due and has not failed (`held::take`), and failing it or
moving it to the mana queue is conditional the same way. So the step of a replaced job changes
nothing, what the runner then records of that step finds no job, and the drainer posts nothing it
has lost.

## In a busy channel

A scheduled message whose time comes while its channel's mana is spent waits in the channel's queue
like anything else ([Channel mana](channel-mana/index.md#waiting-rows)): in its author's item, in
order from when it came due, costing what it would have. Three things are its own:

- **It is never dropped for waiting.** The two-minute limit (`MAX_WAIT`) is for what someone just
  sent and will send again; a scheduled message's author is likely away. The index the drainer walks
  for what has waited too long (`held_message_mana_by_age`) holds no scheduled message.
- **Where anything else would be dropped for want of room, it tries again.** When its author's item
  is full or the queue has no room as it comes due, its job looks again every ten seconds
  (`queue::BUSY_RETRY`, `Waited::NoRoom`); when the queue makes room for someone of higher standing
  by dropping its author's rows, it goes back to its job the same way (`queue::defer`). It gives up
  only when it can never be posted: its author may no longer post there, or it is refused.
- **Its author's apps are told** it waits, by `scheduledMessageChanged`, and show it as "Waiting for
  mana" from the moment it is due, whether it is in the queue or waiting for room there: once when
  it first finds no room (`stuck_since`, `held::note_stuck`), and again as it gets in.

Posting it, once the drainer has paid for it, may fail for a reason that may pass (a database or a
plugin that does not answer). The drainer tries again each tick; if that goes on for two minutes
(`stuck_since`, `held::note_failing`), the message is failed.

## When it cannot be posted

A scheduled message that cannot be posted is kept, failed, for its author rather than dropped
(`held::fail_held`): `failure` says why, and it leaves the mana queue if it was there. It stays
until its author gives it another time, sends it now, or deletes it.

| Case                                                                                           | What happens                                                                             |
| ---------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Refused when posted (the right to post there lost, a plugin refuses it, a file of it deleted)  | Failed, with the refusal's own words                                                     |
| Fails ten times for a reason that may pass (`held::MAX_ATTEMPTS`, thirty seconds apart)        | Failed                                                                                   |
| Its author can no longer see the channel (it was deleted, hidden from them, or a DM they left) | Dropped, told by `heldMessageFailed`: they could never find it to delete                 |
| Its author left, was removed, or was banned; their account was banned or deleted               | Dropped with everything they hold there (`held::drop_held_by`), each told why            |
| Failed thirty days ago (`FAILED_KEPT`)                                                         | Deleted by the hourly `sweepExpired` job, told as a deletion (`scheduled::sweep_failed`) |

## Deleting one

`DELETE` removes the row and its job (`held::cancel`), told by `heldMessageCancelled`. Its files are
its author's to send another way, and their app may hold only the records, not the files to upload
again, so each is counted as confirmed afresh (`attachment::renew_unsent`) and kept a day more, as a
new upload is ([Unsent attachments](attachment-previews/unsent-attachments.md)).

## Events

All to the author alone (`EventScope::User`), in the transaction that makes the change:

| Event                     | When                                                                                              |
| ------------------------- | ------------------------------------------------------------------------------------------------- |
| `scheduledMessageChanged` | It was scheduled, given another time, began waiting for mana, or failed. Carries the whole record |
| `heldMessagePosted`       | It was posted, naming the message                                                                 |
| `heldMessageCancelled`    | Its author deleted it, or the sweep did                                                           |
| `heldMessageFailed`       | It was dropped rather than kept                                                                   |

## What removes rows

- A scheduled message goes when it is posted, deleted, or dropped.
- A failed one is kept thirty days, then swept.
- One author keeps at most a hundred, so the table grows with accounts, not with time.
- A row removed with its channel or its author by cascade leaves its job, which finds no row when it
  comes due and ends.

## Its files

A file in a scheduled message is unsent until the message is posted, and is held from the sweep of
unsent attachments for as long as its row stands. Only its uploader reads it meanwhile
(`attachment::read_attachment`).

## When access is given or taken away

1. **Who can observe it, and by which routes?** Only its author: the reads above answer for the
   caller alone (another's id is not found), and its events go to their user subject. No push, no
   search, no link. Once posted it is a message like any other.
2. **What decides it, and where is that checked?** Scheduling takes what posting takes
   (`message::check_posting`, whose `channel_access` proves the author may send there). Posting it
   checks again, in the posting's transaction, with the author's permissions of that moment.
3. **When the deciding permission is lost?**
   - Losing the right to send (a role's permissions, a role taken or deleted, an override, a move, a
     category deleted) leaves the row. When it comes due it is refused and kept failed, or dropped
     if its author can no longer see the channel.
   - Leaving, removal, a ban, a group DM left, a ban from the deployment, and an account deleted
     drop everything the author holds there by a `dropHeldBy` job saved in the change's transaction,
     scheduled messages among it, each with its job.
   - A sign-out or a password change ends the session, not the message: it is posted at its time if
     its author may still post. It is theirs, written while signed in, as a held message is.
   - The client's store holds only its author's own, and lets each go by the events above.
4. **When it is gained?** Nothing is shown to anyone new. An author who regains the right to post
   before the time has their message posted; one that already failed stays failed until they send
   it.
5. **Does every path that changes it announce it?** Scheduling, another time, sending now, joining
   the mana queue, failing, posting, deleting, dropping, and the sweep each publish. A cascade (the
   channel or the account removed) does not: the channel's removal takes it from nowhere the client
   shows it, and a deleted account has no client.
6. **Is it published inside the transaction that makes the change?** Yes; a rollback is answered by
   `userResync`.

## What a client already holds

1. **Who holds this record?** Its author's apps, from the bootstrap read and
   `scheduledMessageChanged`.
2. **Is every field changed only through an event?** `sendAt`, `status`, and `failure` change by
   `scheduledMessageChanged`, which carries the whole record, from a request, the release job, the
   mana queue's drainer, and the failure paths alike. `content`, the files, and what it replies to
   never change.
3. **Is the caller's own state announced to their other devices?** Every event goes to the user's
   subject. The device that made the change keeps the response only where the stream has not told of
   the message yet (`RecordStore.fillScheduledMessage`), and leaves later changes to the events.
4. **Counts?** None is kept; the client counts the records it holds.
5. **What does a REST read overwrite?** The list is read only at bootstrap, whose events are held
   and applied after it. A response that arrives after the message's posting or deletion was told
   does not bring it back (`#settledHeld`).
6. **What does the client infer from a parent's deletion?** Nothing: the server drops a deleted
   channel's scheduled messages as each comes due and tells of each.
7. **What does a catch-up leave out?** Nothing of its own; user-scoped events are replayed whole.
8. **Outside the stream?** Nothing.

## Testing

- `scripts/check_permissions.py`, `scheduled_messages`: two of the author's devices and a bystander
  across scheduling, another time, a failure, posting, deletion, the limit, and removal.
- `scripts/check_mana.py`: one whose time comes in a busy channel waits past two minutes, then
  posts.
