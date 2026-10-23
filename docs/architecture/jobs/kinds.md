# Kinds

Each `JobKind` decides what its job does, step by step (see [Steps](steps.md)). Classes are
described under [Claiming and classes](claiming.md#classes).

## Summary

| Kind                                                | Class                                              | Recurs                                                         | What it does                                                                                                          |
| --------------------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| [`closePoll`](#closepoll)                           | interactive                                        |                                                                | Closes one poll at its deadline.                                                                                      |
| [`reapVoice`](#reapvoice)                           | normal                                             | every fifteen seconds                                          | Ends silent and lonely calls, clears spent rings.                                                                     |
| [`pruneFailedJobs`](#prunefailedjobs)               | maintenance                                        | daily                                                          | Deletes old given-up jobs and previews no server made.                                                                |
| [`sweepSignIns`](#sweepsignins)                     | maintenance                                        | hourly                                                         | Deletes expired sessions and sign-ins.                                                                                |
| [`sweepUploads`](#sweepuploads)                     | maintenance                                        | hourly                                                         | Deletes stale staging objects, unsent attachments, and unconfirmed reservations.                                      |
| [`moveEvidence`](#moveevidence)                     | normal                                             | every five seconds                                             | Moves evidence off the anonymous read path.                                                                           |
| [`purgeRole`](#purgerole)                           | normal                                             |                                                                | Removes a deleted role's rows.                                                                                        |
| [`shutOut`](#shutout)                               | urgent                                             |                                                                | Signs out users from elsewhere whose homes are no longer admitted.                                                    |
| [`purgeCustomEmoji`](#purgecustomemoji)             | normal                                             |                                                                | Removes a deleted custom emoji's rows.                                                                                |
| [`forgetPluginScope`](#forgetpluginscope)           | normal                                             |                                                                | Deletes what plugins kept in a deleted scope.                                                                         |
| [`retirePlugin`](#retireplugin)                     | normal                                             |                                                                | Takes a removed plugin's notes and account away.                                                                      |
| [`purgePlugin`](#purgeplugin)                       | bulk                                               |                                                                | Deletes everything a removed plugin kept.                                                                             |
| [`recheckAllCalls`](#recheckallcalls)               | urgent                                             |                                                                | Rechecks every call on the deployment.                                                                                |
| [`confirmStanding`](#confirmstanding)               | normal                                             | every `standing_interval_seconds`, at least every five minutes | A federation standing pass.                                                                                           |
| [`makeDigests`](#makedigests)                       | bulk                                               | every minute                                                   | Makes due daily digests.                                                                                              |
| [`sendEmail`](#sendemail)                           | interactive, normal, or bulk, by the mail          |                                                                | Sends one piece of mail.                                                                                              |
| [`queueNewsletter`](#queuenewsletter)               | bulk                                               |                                                                | Queues a newsletter post's mail.                                                                                      |
| [`deleteMessagesBy`](#deletemessagesby)             | normal                                             |                                                                | A ban's deletion window.                                                                                              |
| [`makePicturePreview`](#makepicturepreview)         | interactive, or bulk for what the migration queued |                                                                | Makes one picture's preview.                                                                                          |
| [`makeVideoPoster`](#makevideoposter)               | as `makePicturePreview`                            |                                                                | Takes one video's poster.                                                                                             |
| [`releaseHeldMessage`](#releaseheldmessage)         | interactive                                        |                                                                | Posts one held message.                                                                                               |
| [`drainManaQueue`](#drainmanaqueue)                 | interactive                                        |                                                                | Posts what waits for one busy channel's mana.                                                                         |
| [`firePluginTimer`](#fireplugintimer)               | normal                                             |                                                                | Hands a plugin one of its timers.                                                                                     |
| [`sweepExpired`](#sweepexpired)                     | maintenance                                        | hourly                                                         | Deletes ended community bans, old plugin notices, old voice failure reports, and old steps of the database's latency. |
| [`recountMembers`](#recountmembers)                 | maintenance                                        | hourly                                                         | Recounts each community's members.                                                                                    |
| [`recordStats`](#recordstats)                       | maintenance                                        | hourly                                                         | Writes today's deployment totals.                                                                                     |
| [`forgetIcon`](#forgeticon)                         | maintenance                                        |                                                                | Deletes one icon nothing uses.                                                                                        |
| [`purgeEvidence`](#purgeevidence)                   | maintenance                                        | daily                                                          | Deletes deleted messages' files past the retention.                                                                   |
| [`fitStreams`](#fitstreams)                         | normal                                             | every fifteen seconds                                          | Sizes the NATS streams to their settings and NATS's memory.                                                           |
| [`forgetIdentityProvider`](#forgetidentityprovider) | normal                                             |                                                                | Deletes a removed identity provider's links.                                                                          |

## Polls, calls, and upkeep

### `closePoll`

Closes one poll at its deadline (see [Background tasks](../background-tasks.md)). It is saved with
the poll and keyed by it.

### `reapVoice`

Every fifteen seconds (see [Voice](../voice/index.md)):

- ends the calls of voice servers silent for `session_silence_seconds`,
- ends calls alone for `idle_session_seconds`,
- fifty of each a step, each call in a transaction of its own,
- and clears spent rings.

### `recheckAllCalls`

Rechecks every call on the deployment for a change that touches them all: turning file transfers on
or off (`voice::recheck_all_step`).

- Two hundred seats a step, in order of session and user.
- Other rechecks, of one community, channel, category, or user, are not jobs. They run on their own
  tasks after the change commits, at most `RECHECKS_AT_ONCE` (16) at a time on each server.

### `pruneFailedJobs`

Daily, a thousand a step, deletes:

- jobs given up more than `FAILED_KEPT_DAYS` (30) ago, and
- previews no server has made within seven days (`preview::GIVEN_UP_AFTER`).

### `sweepSignIns`

Hourly, a thousand a step, through `session_expires` and `refresh_token_expires`, deletes:

- sessions an hour after they expire, and
- sign-ins a day after theirs, revoked ones included (revoking expires them).

### `sweepUploads`

Hourly (`media_store::SWEEP_EVERY`), `media_store::sweep_step` deletes:

- staging objects past their upload URLs;
- attachments never sent (`attachment::sweep_unsent`; see
  [Unsent attachments](../attachment-previews/unsent-attachments.md));
- the reservations of uploads never confirmed (`sweep_unconfirmed`); and
- the record of uploads past its window (`upload_quota::prune`).

Deleting one twice is harmless.

### `moveEvidence`

Every five seconds (`attachment::evidence::MOVE_EVERY`), `evidence::move_step` moves the objects of
attachments that became evidence, and their previews, from the anonymous read path to `evidence/`
(see [Evidence](../reports/evidence.md)).

### `purgeEvidence`

Daily, deletes the files of deleted messages, and their link previews' pictures
(`attachment::evidence::purge_step`; see
[Purging evidence](../reports/evidence.md#purging-evidence)), once:

- the deployment setting `evidence_retention_days` has passed, and
- no report case about them is open, or closed within as long.

A hundred of each a step.

### `sweepExpired`

Hourly, a thousand of each a step (`jobs::upkeep::sweep_expired`), deletes:

- community bans past their `until`,
- plugins' notices past a week (`plugin::notice::KEPT`),
- voice failure reports older than `failure_window_seconds`, and
- steps of the database's latency older than their range reaches (see
  [Database latency](../administration/dashboard.md#database-latency)).

Each goes through an index on when it ends or was made, the latency steps through their table's key.
Reads already leave out what it deletes.

### `recountMembers`

Hourly, recounts each community's members into `community.member_count`, which the dashboard lists
and sorts by.

- Five hundred communities a step, in order of id.
- It writes only counts that changed.

### `recordStats`

Hourly, writes today's row of `deployment_stats` (UTC) from the latest row and what was made and
deleted since (`admin::current_totals`). The dashboard's overview and growth read it (see
[Dashboard](../administration/dashboard.md)).

### `fitStreams`

Every `nats_memory::FIT_EVERY` (fifteen seconds), makes the two in-memory NATS streams the sizes
their settings and NATS's memory give them (`nats_memory::fit_step`; see
[NATS memory](../nats-memory.md)).

- It holds the settings row's lock while it does, as a change to the sizes does.
- It writes to NATS only when a stream is missing or the wrong size.
- A pass that fails is logged and left to the next, so its period never grows into a backoff.

### `forgetIdentityProvider`

Deletes a removed identity provider's links to accounts, a thousand a step
(`identity_provider::forget_step`), keyed by the provider. Removing a provider saves it (see
[Identity providers](../sign-in-and-security/identity-providers.md#removing-a-provider)). The
provider's row stays, since the sign-ins it made name it.

### `forgetIcon`

Deletes one icon and its picture if nothing uses it (`icon::forget_step`), keyed by the icon. The
database queues it, through the triggers of `aspen_icon_let_go` and `aspen_icon_confirmed`:

- a day after an icon's upload is confirmed, for one never put to use, and
- as soon as a user, community, or the deployment lets go of its icon or is deleted.

## Deleted roles and emoji

### `purgeRole`

Takes a deleted role off its holders and the tags of it, a thousand a step, then deletes it
(`role::purge_step`).

Deleting a role (`role::retire_role`) does, at once:

1. marks it deleted,
2. takes its permissions, hue, and showing apart away,
3. puts it below every role,
4. deletes its overrides,
5. announces it.

Every read of roles leaves deleted ones out, so it grants nothing and ranks nobody while its rows
go.

### `purgeCustomEmoji`

Takes a deleted custom emoji's reactions off, a thousand a step, then deletes it and its picture,
unless something took the picture up (`custom_emoji::purge_step`).

Deleting an emoji marks it deleted and announces it at once. After that no read lists it, resolves
it, or counts its reactions, and its name is free again.

## Moderation and federation

### `deleteMessagesBy`

A ban's deletion window (`message::queue_deletion_of_recent`). It deletes the banned person's
messages after the window's start and before the ban:

- in the channels the banner could view then, and their threads, or
- anywhere, for a ban from the deployment.

Each step:

1. takes two hundred messages, newest first;
2. deletes them as `message::soft_delete_many` deletes: one statement for the messages, one for
   their echoes, their files kept as evidence, each thread's summary taken down once, and every
   deletion announced together;
3. moves the job's place past them in the same transaction.

The ban answers how many messages it covers.

### `shutOut`

Signs out users from elsewhere (see [Federation](../federation/index.md)), a hundred a step, each in
a transaction of its own. It is saved:

- after a change to a gate or a list, for those whose homes the gates no longer admit, decided per
  home from the gates as each step runs, or
- for everyone of one home suspended for an unvouched key.

### `confirmStanding`

A standing pass while a gate is open (see [Federation](../federation/index.md)), every
`standing_interval_seconds` and at least every five minutes. It:

- asks homes about their users here who are due, a bounded number a pass,
- reads again the documents of the deployments in use, and
- forgets deployments contacted once and never used.

Which deployments are failing is remembered by each server (`StandingBackoff`).

## Email

### `makeDigests`

Every minute, makes the daily digests that are due, twenty a step, each in a transaction of its own
(see [Email](../email/index.md)). Only servers that send mail run it.

### `sendEmail`

Sends one piece of mail (see [Email](../email/index.md)), on servers that send. Its class is
interactive, normal, or bulk, by the mail.

- A failure for now waits a minute, doubling.
- After eleven attempts the piece is given up rather than kept.

### `queueNewsletter`

Queues a sent newsletter post's mail, a thousand subscribers a step. Each batch's jobs are saved in
one statement (`jobs::enqueue_many`).

## Attachment previews

See [Attachment previews](../attachment-previews/index.md).

### `makePicturePreview`

Makes one picture's preview, keyed by its attachment. Its class is interactive, or bulk for what the
migration queued.

- It runs on servers whose `[media.previews]` has `make` on, at most `concurrency` previews at once.
- Its payload holds until when the messages holding the attachment wait for it (`holdUntil`).
- A failure waits a minute, doubling, six times.
- One no server made within seven days is deleted by [`pruneFailedJobs`](#prunefailedjobs).

### `makeVideoPoster`

Takes one video's poster, as `makePicturePreview` makes a picture's, with the same class. It runs on
the servers among those that can run `ffmpeg` and `ffprobe`.

### `releaseHeldMessage`

Posts one held message, keyed by it, once:

- no preview job holds it, and
- no message its author sent before it is still held.

A failure waits thirty seconds. The tenth failure drops the message and tells its author why.

A message whose channel is busy when it is released waits for the channel's mana instead (see
[Channel mana](../channel-mana/index.md)): its row becomes one waiting for mana, and the job is
done.

### `drainManaQueue`

Posts what waits for one channel's mana as the channel recharges, keyed by the channel
(`mana::queue::drain_step`; see [Channel mana](../channel-mana/index.md#draining)). It is saved with
the first row to wait there.

- Each step ticks at 5 Hz for ten seconds, then hands the job back with the author it drew as its
  progress, so its lease renews and any runner may take it up.
- Each tick drops what has waited two minutes, then spends what the pool earned on the
  highest-priority authors waiting: the higher ranked, then the longer there, then drawn at random.
- It deletes its own job only while nothing waits there, in one statement, so a row saved as it
  finishes saves a new job.

### `dropHeldBy`

Drops everything someone has held where they can no longer post (`message::held::drop_held_by_step`;
see [Held messages](../attachment-previews/held-messages.md#when-posting-fails)). Saved, in the
change's transaction, by ending a community membership (`community::end_membership`: leaving,
removal, a ban), leaving a group DM, a ban from the deployment (the person and their bots), and
deleting an account, and only when they have something held there.

- Each step drops a hundred, oldest first, each told to its author in the language it was sent in,
  and goes on while steps come back full.

## Plugins

See [Plugins](../plugins/index.md).

### `forgetPluginScope`

Deletes what plugins kept in a deleted community, channel, or account, and the timers set there
(`plugin::storage::forget_step`), in this order:

1. the scope's own values;
2. its channels' values (or a channel's threads'), a batch of five hundred channels at a time by id,
   each batch's values taken off their owner's share in the statement that deletes them;
3. a community's or an account's share.

### `retirePlugin`

For a removed plugin (`plugin::install::retire_step`):

- takes its notes away (no read shows them once it is removed), and
- takes its account out of its communities fifty at a time, announced as any member leaving is.

It also runs on its account alone, for a plugin installed again without one.

### `purgePlugin`

Deletes everything a removed plugin kept, a thousand rows of each table a step
(`plugin::install::purge_step`).

### `firePluginTimer`

Hands a plugin one of its timers when it falls due (`plugin::timer::fire_step`), keyed `plugin/key`.

- A timer set again replaces the job under a new id.
- A timer of a plugin that is off waits until it is turned on.
- Three failures give it up.
