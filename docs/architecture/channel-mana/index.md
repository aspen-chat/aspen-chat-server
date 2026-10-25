# Channel mana

Channel mana bounds how fast a whole channel fills, however many people post in it (`app::mana`).
Each channel has a budget, which posting spends and which comes back over time. What the budget
cannot pay for waits its turn rather than being refused, in a queue that serves members by rank,
then by how long they have belonged, so a raid of new accounts waits behind the regulars. It is
invisible until a channel is busy: people see a bar only while the budget is below 60%.

Each person's own posting limits stay (`rate_limits.toml`), so one person is held by their own
limits and a crowd by the channel's mana. [Why it is built this way](design-notes.md)

## Where it lives

| Part                             | Where                                                                                                                                                                                                                  |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Costs, pools, spending, deciding | `app::mana`                                                                                                                                                                                                            |
| The queue and its drainer        | `app::mana::queue`, the `drainManaQueue` job                                                                                                                                                                           |
| The live level                   | `app::mana::live`, `EphemeralEvent::Mana`, core NATS `aspen.mana`                                                                                                                                                      |
| Waiting rows                     | `held_message` (`waiting = 'mana'`), `app::message::held`                                                                                                                                                              |
| Gates                            | `held::post` (messages and thread replies), `message::invoke_command`, `message::update_message`, `poll::create_poll`, `thread::request_echo`, and `held::release_step` for a message released from its preview's hold |
| Settings                         | `deployment_settings` (`mana_max_recharge`, `mana_max`, `mana_attachment_cost`); `channel` (`mana_recharge`, `mana_max`)                                                                                               |
| Permission                       | `Permissions::BYPASS_CHANNEL_MANA` (bit 47)                                                                                                                                                                            |
| Client                           | `ManaBar` along the top of the typing line, `HeldMessages` above the message box, `ChannelManaDialog`, the dashboard's policies                                                                                        |

## Pools

| Conversation                       | Pool    | Settings             |
| ---------------------------------- | ------- | -------------------- |
| Community text channel             | Its own | The channel's        |
| Thread                             | Its own | Its parent channel's |
| Group DM, and a thread in one      | Its own | The deployment's     |
| One-to-one DM, and a thread in one | None    |                      |
| Voice and plugin channels          | None    |                      |

A pool is a Valkey hash, `mana:{channel}`, of `level` and `tick` (the tick it is as of: 200 ms each
since 1970). One Lua script (`mana::POOL`) brings it up to date and spends from it in one step, on
Valkey's clock, so nothing runs while a channel is quiet and two servers cannot both spend its last
mana.

- **Units:** levels are in thousandths of mana, so every rate a second divides evenly into 5 Hz
  ticks: 50 mana a second is 10,000 thousandths a tick.
- **Recharge:** the channel's `mana_recharge` a second, or the deployment's `mana_max_recharge`
  where it sets none, and never more than the deployment's.
- **Capacity:** the channel's `mana_max`, or the deployment's (20,000) where it sets none, and never
  more than the deployment's.
- **Floor:** nothing takes a pool below `-capacity / 2`.
- **Full pools keep no key:** a pool that fills is deleted, and one Valkey does not hold is full. A
  key also expires a second after it would have filled.
- **Loss:** a pool lost from Valkey (an eviction, a restart without persistence) comes back full.

## What things cost

| Part                   | Cost                                  |
| ---------------------- | ------------------------------------- |
| Each glyph of the text | 1                                     |
| Each attachment        | `mana_attachment_cost`, 30 by default |

- **A glyph is a grapheme cluster** (`mana::glyphs`, with `unicode-segmentation`): what a reader
  sees as one character. `é`, `中`, `👍🏽`, `👨‍👩‍👧` and `🇯🇵` cost 1 each, in every script alike.
- **A stacked cluster** of more than ten scalars (`GLYPH_SCALARS`) costs one more for each scalar
  past ten: a chain of emoji joined by zero-width joiners, or a letter under a heap of combining
  marks, costs about what its characters would apart. The longest emoji sequence Unicode recommends
  is ten scalars, so every emoji, and every script's letters, still cost 1.
- **Tokens:** a tag (`<@id>`, `<@&id>`), a custom emoji (`<:id>`), and `@everyone` cost 1 each,
  however long their stored form.
- **Markdown markers** cost what they are, a glyph each.
- **Counted on the text as sent**, before the channel's plugins decide it.
- **A slash command** costs its invocation's text, as its message shows it, and its attachments.
- **An edit** costs the glyphs it adds beyond the message's length before, and its added attachments
  (`mana::edit_cost`). One that shortens or rewords costs nothing and applies at once, even while
  others wait.
- **An edit that changes nothing** (the same text and the same attachments) is dropped, and answered
  with the message as it stands. The apps do not send one.
- **A poll** costs the glyphs of its question and answers, one line each.
- **An echo** to a thread's parent costs its reply's cost, from the parent's pool.

## Who spends

| Author                                                        | Spends | Waits              |
| ------------------------------------------------------------- | ------ | ------------------ |
| A person, of this deployment or another                       | Yes    | Yes                |
| A bot                                                         | Yes    | Yes                |
| Anyone holding Bypass channel mana there                      | Yes    | No: posted at once |
| The system account                                            | No     |                    |
| A plugin posting as itself (`Deferred::Send`, cards, notices) | No     |                    |
| What the server posts (a poll's closing, calls, missed calls) | No     |                    |

- **Bypass channel mana** is a channel permission, adjustable by override, in the Moderator and
  Admin templates. A migration gave it to every role that held Manage messages. A thread takes it
  from its parent.
- **A bypassing message still spends**, down to the floor, so the bar shows what was said.
- **A bot** that must post freely is given Bypass through its role.
- **DM recipients** hold every channel permission but this one (`Permissions::IN_DM`), so no one in
  a group DM bypasses.
- **The system account and plugins** post through `message::post` directly, past every gate.

## Deciding

Every gate first checks that the author may post there as posting would (`check_posting`, or
`check_first_reply` for a reply that makes its thread, or the endpoint's own checks), with its
attachments and what it replies to, so only what could be posted spends a channel's mana. Then
`mana::decide_for` answers:

1. **No pool, or nothing to pay:** go ahead.
2. **Bypass:** spend, down to the floor, and go ahead.
3. **Something already waits in the pool:** wait too, in the author's item, so nothing jumps the
   queue.
4. **The pool holds the cost:** spend, and go ahead.
5. **The pool is full but cannot pay** (a message larger than all it holds): spend anyway, into
   debt, down to the floor, and go ahead.
6. **Otherwise:** wait.

What goes ahead is posted as it would be anyway; spending comes before its transaction, so one
refused after spending (by a plugin, say) has spent for nothing, which the next ticks return. What
waits is answered `202 Accepted` with the held row.

- **A thread's first reply always goes ahead:** the thread it makes has a new, full pool.
- **An echo made with its reply** spends the parent's pool too, decided only once the author is
  known to hold Send messages in the parent, as posting the echo takes. When the parent cannot pay,
  the reply is posted alone, from the thread's pool, and its echo waits in the parent's queue as an
  item of its author's (`held::post_now`). An echo the parent's queue cannot take then is not made,
  and its author may echo the reply later.
- **An echo asked for later** (`PUT /messages/{message}/echo`) waits in the parent's queue the same
  way.
- **A message released from its preview's hold** asks the channel too. When the channel is busy, its
  row becomes one waiting for mana (`queue::wait_held`), without the bounds below, since it was
  accepted when sent.

## Waiting rows

What waits is a held message ([Held messages](../attachment-previews/held-messages.md)), its
`waiting` being `mana`:

| Column                     | Holds                                                                                                        |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `channel`                  | The pool it pays from: for an echo, the thread's parent                                                      |
| `kind`                     | `message`, `command`, `edit`, `poll`, or `echo` (`HeldKind`)                                                 |
| `target`                   | The message an edit changes, or the reply an echo copies                                                     |
| `payload`                  | A command's invocation, an edit's change (`message::WaitingEdit`), or a poll as it was asked for             |
| `content`                  | A message's text, a command's as shown, an edit's new text, or a poll's question, for its author's apps      |
| `cost`                     | What it spends, in mana                                                                                      |
| `reply_to`, `tags_replied` | What a message replies to, checked as it was sent, and whether it tags its author ([Replies](../replies.md)) |

- **Its author's apps** list it with the rest (`GET /users/@me/held-messages`), saying what it waits
  for (`waiting`) and what it is (`kind`).
- **Posted:** `heldMessagePosted` names the message it became, or the message an edit changed.
- **Dropped:** `heldMessageFailed` says why.
- **Cancelled:** `DELETE /users/@me/held-messages/{held}` cancels it, announced by
  `heldMessageCancelled`.
- **An edit's target or an echo's reply deleted:** the row goes with it (`ON DELETE CASCADE`). The
  apps let it go when they hear the message was deleted.

### Items and their bounds

An author's waiting rows in one pool are one item. Its rows post in the order sent.

| Bound                | Value                                                                                                  | Past it                                                                              |
| -------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ |
| An item              | 50 rows (`MAX_ITEM_MESSAGES`), or 2,000 mana in all (`MAX_ITEM_MANA`), its first row whatever it costs | `429` `channelBusy`, `Retry-After` 10 seconds (`BUSY_RETRY`)                         |
| A pool's items       | 1,000 (`MAX_ITEMS`)                                                                                    | Room is made for an author above the last one waiting (below); anyone else, the same |
| How long a row waits | 2 minutes (`MAX_WAIT`)                                                                                 | Dropped, its author told the channel was too busy                                    |

### Queue order

Items go by their authors' `Standing` (`queue::standings_of`, one query for every author asked of).
First by `Priority`, higher first:

1. **Rank** in the channel's community, as everywhere else it is ranked
   (`CommunityAccess::role_rank`): the owner above every role, otherwise their highest role's
   position, 0 with the everyone role alone. A thread's is in its parent's community. Everyone in a
   group DM is 0.
2. **Tenure**, in whole days since they joined the community (`community_user.joined_at`), or were
   added to the group DM (`dm_recipient.joined_at`).

Then, among those equal in both, by their **tie**, lowest first: HMAC-SHA256 of the channel's id,
the author's id, and their username, keyed with the deployment's `manaOrder` secret
(`server_secret::MANA_ORDER`, made by the first server to start, as its other secrets are). The
order is arbitrary but the same on every server and every tick, so it needs no checkpoint, and no
one outside the servers can work it out.

An author who no longer belongs there has the lowest priority. Holders of Bypass channel mana never
wait, so they are never in the queue.

- **A full queue:** when 1,000 authors wait and someone else sends, `queue::make_room` reads the
  standings of them all. If the sender's priority is strictly above that of the last of them in
  queue order, every waiting row of that author is dropped, each told the channel made room for
  someone of higher rank or longer here (`manaMadeRoom`), and the sender waits. Otherwise the sender
  is refused with `channelBusy`. No one is ever dropped for someone of lower priority, or for an
  equal. It is checked before the row is saved, apart from its transaction, since dropping announces
  each row; two senders at once may both find room, and the queue holds a few more.

### Thirds

While anything waits, what the pool holds is not spent by whoever asks first but credited to the
first three authors in queue order (`mana::share`, the `SHARE` script):

| Waiting       | First            | Second    | Third     |
| ------------- | ---------------- | --------- | --------- |
| One           | All three thirds |           |           |
| Two           | Two thirds       | One third |           |
| Three or more | One third        | One third | One third |

- **What is split:** whatever the pool holds above zero when the queue forms, and each recharge
  after, split into three equal portions (`mana::WEIGHTS`), the rounding's remainder to the first.
  The fourth and later earn nothing until they move up. A pool in debt pays its debt back first.
- **Paying:** an author's oldest row is posted once their credit covers its cost (`mana::pay_turn`),
  the cost coming out of their credit. Credit left over stays theirs while they still have rows
  waiting.
- **Credit goes back to the pool** when its holder is no longer among the first three: their last
  row posted, cancelled, or dropped (at `MAX_WAIT`, or when they could no longer post), or pushed
  down by someone of higher priority arriving. It is shared out again at the same tick.
- **A row larger than a share:** once an author's credit reaches their share of the capacity (a
  third of it for each third they are credited) while the pool is not in debt, their row posts
  however much it costs, with all their credit and the rest from the pool, into debt down to the
  floor. With one waiting this is the full pool's rule; a row costing more than the author can ever
  be credited would otherwise never post.
- **Where credit lives:** beside the pool in Valkey, a hash `mana:{channel}:credit` of user ids to
  thousandths. The pool's scripts treat it as the pool's own: the level never rises above the
  capacity less what is credited, and the level told to viewers includes it, so the bar rises and
  falls as it would without the queue. One script call shares and another pays, each atomically on
  Valkey's clock, and only the pool's one drainer calls them, so credit is consistent whichever
  server runs the drainer. It expires a minute after the drainer last touched it, and a pool whose
  credit is lost has that mana back no sooner than its recharge returns it.

## Draining

One job drains a pool while anything waits there: `drainManaQueue`, keyed by the channel, so one
runner holds it anywhere in the fleet (`queue::drain_step`). It is saved with the first row to wait
there.

- **Ticking:** a step ticks at 5 Hz (`TICK`) for ten seconds, then hands the job back
  (`Outcome::Progress`), so its lease renews and any runner may take it up. It carries nothing from
  step to step, since the credit lives in Valkey. A drainer takes a place of the `interactive` class
  like any job, so many busy channels at once wait their turns for places.
- **Standings** are read as each author is first seen waiting in a step, and kept for the step, so a
  role given or taken counts within ten seconds.
- **Each tick** first drops what has waited `MAX_WAIT`, a hundred at a time, telling each author.
  Then it reads each item's oldest row, puts them in queue order, and, up to ten times
  (`ROUNDS_PER_TICK`): shares what the pool has earned among the first three, and posts the oldest
  row of each whose credit can pay for it.
- **Higher priority earns more, not everything:** the first in the queue is credited at least a
  third of all the pool earns, and two thirds or all of it when fewer wait. A raid of new accounts
  holds at most two of the three places while one regular waits, and none while three do, so it is
  mostly dropped at `MAX_WAIT`; and one regular's costly row does not hold back everyone below them
  while it is paid for.
- **Posting a row** is posting as its kind would be (`held::post_waiting`): a message or a command
  by `message::post`, an edit by `update_message`, a poll by `create_poll`, each taking the row in
  its own transaction; an echo by `echo_reply`, its row going after, since echoing twice makes one
  echo. Permissions, blocks, and plugins decide it then; one that can no longer be posted is
  dropped, its author told why, and one that fails for a reason that may pass is left to wait.
- **Checked before paying:** before a row is paid for, the drainer checks that its author may still
  post there (`held::drop_if_unpostable`: `may_post`, or for an edit their access to the channel).
  One they no longer may is dropped unpaid, so what will be refused spends nothing of the channel's
  mana.
- **Finishing:** a drainer that finds nothing waiting gives every credit back to the pool, then
  deletes its own job, in one statement, only while nothing waits (`finish_if_empty`), so a row
  saved at that moment sees the job gone and saves another (`ensure_drainer`).
- **A channel that can no longer have a pool** (made something else, or gone) has what waits there
  posted without paying.
- **Capacity bounds a long queue:** a thousand items of 100 mana each is 100,000 mana, over half an
  hour at 50 a second, so most are dropped at `MAX_WAIT`, the last in queue order first.

## The bar

- **Telling viewers:** a spend publishes the pool's level as a relay on core NATS `aspen.mana`
  (`live::changed`), routed exactly as typing is (`event_feed::relayed_events`): to connections
  viewing the channel (`viewing`) that may read it, as an `ephemeral` frame, never replayed. The
  frame (`EphemeralEvent::Mana`) holds `channelId`, `level` (thousandths, what the queue's authors
  are credited included, below zero in debt), `max`, `recharge`, `at`, and `queued`, whether
  anything waits there, so its priority queue is in force (read as the frame is told).
- **Twice a second at most:** the first spend in each half second (`live::EVERY`) sets
  `mana:{channel}:told` and is told; a server whose spend fell inside the window tells the level as
  it is when the window ends, so the last spend of a burst always reaches viewers.
- **Something begins waiting, or the queue empties:** the level is told then too
  (`queue::tell_level`), since nothing may have been spent, as when a long message waits for the
  pool to recharge enough to pay for it.
- **Starting to view:** each event stream connection's task (`live::Viewer`) tells it the level of
  each channel it starts to view whose pool is below full and that it may read, so a bar already
  showing appears at once.
- **In the app:** the store keeps the latest level of up to 32 channels (`RecordStore.mana`), and
  the bar draws the level between frames from `at` and `recharge`, rising steadily at 15 frames a
  second (`manaDrawnAt`). It shows once the level falls below 60% of `max` (`MANA_SHOW_BELOW`), or
  while anything is queued, and goes once the pool is full. It is labelled "Channel Mana", or, while
  anything is queued, "Channel Mana depleted, message priority queue in force". It lies along the
  top of the typing line, so the conversation never moves for it.
- **A lost connection** forgets every level; viewing a channel anew is told it again.

## Top senders

Beside the bar's label, "View top senders" opens who spent most of the channel's mana in the past
hour (`GET /channels/{channel}/mana/top-senders`, `mana::read_top_senders`).

- **Counted as it is spent:** the pool's script adds each spend to its sender in whole mana, in a
  sorted set of that minute's (`mana:{channel}:sent:{minute}`), kept 61 minutes. Posting at once,
  with Bypass, and paying for a waiting row all count; what is refused or dropped spends nothing, so
  counts nothing. A thread counts its own, as its pool is its own.
- **Read:** a second script sums the last 60 minutes' sets into `mana:{channel}:top`, kept five
  seconds, and answers its first 100 (`mana::TOP_SENDERS`), the most first, each with their mana.
  However many people have the list open, a channel's hour is summed at most every five seconds, and
  the sum's work grows with how many sent in the hour.
- **Who sees it:** anyone who may read the channel (`channel_access`); `include=users` sideloads the
  senders, few of whom a channel's readers may know in a raid. Where the channel has no pool the
  list is empty, and while Valkey cannot be read it is empty too, as mana that cannot be counted is.
- **In the app:** `TopSendersDialog` reads it again every five seconds while open. Each sender the
  caller may ban there (`useMemberModeration`: Ban members, over someone ranked below them, not the
  owner) has a Ban button, opening the ban dialog set to delete their messages from the past hour.
- **Rate limit:** 30 a minute for each person (bursts of 10), 600 a minute for each channel.

## Settings

| Setting                | Where                        | Default                              | Who changes it             |
| ---------------------- | ---------------------------- | ------------------------------------ | -------------------------- |
| `mana_max_recharge`    | Deployment settings          | 50 a second                          | Manage deployment settings |
| `mana_max`             | Deployment settings          | 20,000                               | Manage deployment settings |
| `mana_attachment_cost` | Deployment settings          | 30                                   | Manage deployment settings |
| `mana_recharge`        | The channel (`manaRecharge`) | The deployment's `mana_max_recharge` | Manage channels            |
| `mana_max`             | The channel (`manaMax`)      | The deployment's `mana_max`          | Manage channels            |

- **The deployment's are ceilings:** `POST /channels` and `PATCH /channels/{channel}` refuse a
  channel's above them, or below 1, with `validation` and a message naming the limit
  (`mana::check_channel_settings`).
- **Clamped where used:** a channel set above a deployment setting later lowered is held to it at
  once (`PoolSettings::of`).
- **`null` follows the deployment's**, as it changes.
- **The deployment's are public:** `GET /deployment` gives them as `mana`, for the channel's
  settings to show.
- **The deployment's** must be at least 1 (`manaSettingZero`).

## Rate limits

What one person may post (`rate_limits.toml`): messages and commands 50 per 5 seconds in a channel
(bursts of 100) and 600 a minute (bursts of 200); thread replies 200 a minute (bursts of 100); edits
300 a minute (bursts of 100). A whole channel's flood has no limit here: its mana is that. Bots have
ten times a person's (`bot_multiplier`).

## When Valkey is down

Mana cannot be counted, so everything posts as if every pool were full: it fails open, and the
server logs it twice a minute at most. What already waits is paid for as it is posted, and so posts
too.

## When access is given or taken away

1. **Who can observe it:** a pool's level, the connections viewing the channel that may read it (as
   typing, routed by the channel's governing permissions); who spent it, anyone who may read the
   channel, by REST alone; a waiting row, only its author (user-scoped events,
   `GET /users/@me/held-messages`); a channel's settings, whoever reads the channel's record.
2. **What decides it:** reading the channel for the level, checked by the event feed's routing and,
   for the first frame, by `channel_access` (`live::for_viewer`); authorship for waiting rows;
   Manage channels for a channel's settings; Bypass channel mana, read by `channel_access` as each
   gate decides.
3. **When the deciding permission is lost:**
   - Leaving, removal, or a ban from a community, leaving a group DM, a ban from the deployment, and
     deleting an account drop what the person has waiting there at once, by a `dropHeldBy` job saved
     in the same transaction
     ([Held messages](../attachment-previews/held-messages.md#when-posting-fails)). They then rank
     lowest and spend nothing meanwhile, and what they were credited goes back to the pool at the
     drainer's next tick once their last row there is gone.
   - Any other loss of the right to post there (a role or override changed, a role taken or deleted,
     a move, a category deleted, the channel deleted) is found as the drainer reaches the row,
     before it pays: dropped unpaid, with `heldMessageFailed`. A row reached only later is checked
     by the posting itself too, or goes with its author, channel, or target.
   - Losing Bypass affects only what is sent after.
   - The level stops reaching a connection as it loses the channel, as typing does, and its top
     senders are refused (`404`) from then on; an open list stops refreshing at its next read.
4. **When it is gained:** a connection that starts viewing a busy channel is told its level. A new
   holder of Bypass goes ahead with their next message.
5. **Every path announces it:** spends publish the level; waiting rows are announced as posted,
   dropped, or cancelled; a channel's settings go out as `ChannelEvent::Update`, the deployment's
   through the settings' own announcement.
6. **Inside the transaction:** a waiting row and its drainer's job are saved in one transaction;
   posting a waiting row takes it in the posting's transaction. Spending is in Valkey, outside every
   transaction.

`scripts/check_permissions.py` checks that a member removed while their message waits has it
dropped, and that a member who loses the channel stops hearing its level.

## Testing

- **Unit tests:** glyph counting and costs, settings and their ceilings, and the thirds' sharing,
  paying, and debt against the Valkey of `docker-compose.yaml` (or `VALKEY_URL`) (`app::mana`); the
  queue's order and its keyed ties (`app::mana::queue`); the app's recharge arithmetic, store, and
  held edits (`client/packages/protocol/test/mana.test.ts`, `sync.test.ts`).
- **The `raid` benchmark scenario** (`aspen-bench`, see [Benchmarking](../benchmarking.md)) has a
  tenth of a 2,000-member community flood one channel, measuring how long everyone else's messages
  there wait and how many of either side's are held, refused, or dropped.
- **`scripts/check_mana.py`** drives a stack through it: spending and waiting, Bypass, draining a
  higher rank first, the bar's frames, edits (waiting, made, and changing nothing), polls, echoes,
  the two-minute limit, cancelling, and the bounds.
