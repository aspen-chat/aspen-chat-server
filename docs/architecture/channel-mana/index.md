# Channel mana

Channel mana bounds how fast a whole channel fills, however many people post in it (`app::mana`). Each channel has a budget, which posting spends and which comes back over time. What the budget cannot pay for waits its turn in a queue shared fairly among the people waiting, rather than being refused. It is invisible until a channel is busy: people see a bar only while the budget is below 60%.

Each person's own posting limits stay (`rate_limits.toml`), so one person is held by their own limits and a crowd by the channel's mana. [Why it is built this way](design-notes.md)

## Where it lives

| Part | Where |
| --- | --- |
| Costs, pools, spending, deciding | `app::mana` |
| The queue and its drainer | `app::mana::queue`, the `drainManaQueue` job |
| The live level | `app::mana::live`, `EphemeralEvent::Mana`, core NATS `aspen.mana` |
| Waiting rows | `held_message` (`waiting = 'mana'`), `app::message::held` |
| Gates | `held::post` (messages and thread replies), `message::invoke_command`, `message::update_message`, `poll::create_poll`, `thread::request_echo`, and `held::release_step` for a message released from its preview's hold |
| Settings | `deployment_settings` (`mana_max_recharge`, `mana_max`, `mana_attachment_cost`); `channel` (`mana_recharge`, `mana_max`) |
| Permission | `Permissions::BYPASS_CHANNEL_MANA` (bit 47) |
| Client | `ManaBar` along the top of the typing line, `HeldMessages` above the message box, `ChannelManaDialog`, the dashboard's policies |

## Pools

| Conversation | Pool | Settings |
| --- | --- | --- |
| Community text channel | Its own | The channel's |
| Thread | Its own | Its parent channel's |
| Group DM, and a thread in one | Its own | The deployment's |
| One-to-one DM, and a thread in one | None | |
| Voice and plugin channels | None | |

A pool is a Valkey hash, `mana:{channel}`, of `level` and `tick` (the tick it is as of: 200 ms each since 1970). One Lua script (`mana::POOL`) brings it up to date and spends from it in one step, on Valkey's clock, so nothing runs while a channel is quiet and two servers cannot both spend its last mana.

- **Units:** levels are in thousandths of mana, so every rate a second divides evenly into 5 Hz ticks: 50 mana a second is 10,000 thousandths a tick.
- **Recharge:** the channel's `mana_recharge` a second, or the deployment's `mana_max_recharge` where it sets none, and never more than the deployment's.
- **Capacity:** the channel's `mana_max`, or the deployment's (20,000) where it sets none, and never more than the deployment's.
- **Floor:** nothing takes a pool below `-capacity / 2`.
- **Full pools keep no key:** a pool that fills is deleted, and one Valkey does not hold is full. A key also expires a second after it would have filled.
- **Loss:** a pool lost from Valkey (an eviction, a restart without persistence) comes back full.

## What things cost

| Part | Cost |
| --- | --- |
| Each glyph of the text | 1 |
| Each attachment | `mana_attachment_cost`, 30 by default |

- **A glyph is a grapheme cluster** (`mana::glyphs`, with `unicode-segmentation`): what a reader sees as one character. `é`, `中`, `👍🏽`, `👨‍👩‍👧` and `🇯🇵` cost 1 each, in every script alike.
- **Tokens:** a tag (`<@id>`, `<@&id>`), a custom emoji (`<:id>`), and `@everyone` cost 1 each, however long their stored form.
- **Markdown markers** cost what they are, a glyph each.
- **Counted on the text as sent**, before the channel's plugins decide it.
- **A slash command** costs its invocation's text, as its message shows it, and its attachments.
- **An edit** costs the glyphs it adds beyond the message's length before, and its added attachments (`mana::edit_cost`). One that shortens or rewords costs nothing and applies at once, even while others wait.
- **An edit that changes nothing** (the same text and the same attachments) is dropped, and answered with the message as it stands. The apps do not send one.
- **A poll** costs the glyphs of its question and answers, one line each.
- **An echo** to a thread's parent costs its reply's cost, from the parent's pool.

## Who spends

| Author | Spends | Waits |
| --- | --- | --- |
| A person, of this deployment or another | Yes | Yes |
| A bot | Yes | Yes |
| Anyone holding Bypass channel mana there | Yes | No: posted at once |
| The system account | No | |
| A plugin posting as itself (`Deferred::Send`, cards, notices) | No | |
| What the server posts (a poll's closing, calls, missed calls) | No | |

- **Bypass channel mana** is a channel permission, adjustable by override, in the Moderator and Admin templates. A migration gave it to every role that held Manage messages. A thread takes it from its parent.
- **A bypassing message still spends**, down to the floor, so the bar shows what was said.
- **A bot** that must post freely is given Bypass through its role.
- **DM recipients** hold every channel permission but this one (`Permissions::IN_DM`), so no one in a group DM bypasses.
- **The system account and plugins** post through `message::post` directly, past every gate.

## Deciding

Every gate first checks that the author may post there as posting would (`check_posting`, or the endpoint's own checks), so only someone who could post spends a channel's mana. Then `mana::decide_for` answers:

1. **No pool, or nothing to pay:** go ahead.
2. **Bypass:** spend, down to the floor, and go ahead.
3. **Something already waits in the pool:** wait too, in the author's item, so nothing jumps the queue.
4. **The pool holds the cost:** spend, and go ahead.
5. **The pool is full but cannot pay** (a message larger than all it holds): spend anyway, into debt, down to the floor, and go ahead.
6. **Otherwise:** wait.

What goes ahead is posted as it would be anyway; spending comes before its transaction, so one refused after spending (by a plugin, say) has spent for nothing, which the next ticks return. What waits is answered `202 Accepted` with the held row.

- **A thread's first reply always goes ahead:** the thread it makes has a new, full pool.
- **An echo made with its reply** spends the parent's pool too. When the parent cannot pay, the reply is posted alone, from the thread's pool, and its echo waits in the parent's queue as an item of its author's (`held::post_now`). An echo the parent's queue cannot take then is not made, and its author may echo the reply later.
- **An echo asked for later** (`PUT /messages/{message}/echo`) waits in the parent's queue the same way.
- **A message released from its preview's hold** asks the channel too. When the channel is busy, its row becomes one waiting for mana (`queue::wait_held`), without the bounds below, since it was accepted when sent.

## Waiting rows

What waits is a held message ([Held messages](../attachment-previews/held-messages.md)), its `waiting` being `mana`:

| Column | Holds |
| --- | --- |
| `channel` | The pool it pays from: for an echo, the thread's parent |
| `kind` | `message`, `command`, `edit`, `poll`, or `echo` (`HeldKind`) |
| `target` | The message an edit changes, or the reply an echo copies |
| `payload` | A command's invocation, an edit's change (`message::WaitingEdit`), or a poll as it was asked for |
| `content` | A message's text, a command's as shown, an edit's new text, or a poll's question, for its author's apps |
| `cost` | What it spends, in mana |
| `credit` | What its author's item has been given towards it, in thousandths, held on the item's oldest row |

- **Its author's apps** list it with the rest (`GET /users/@me/held-messages`), saying what it waits for (`waiting`) and what it is (`kind`).
- **Posted:** `heldMessagePosted` names the message it became, or the message an edit changed.
- **Dropped:** `heldMessageFailed` says why.
- **Cancelled:** `DELETE /users/@me/held-messages/{held}` cancels it, announced by `heldMessageCancelled`.
- **An edit's target or an echo's reply deleted:** the row goes with it (`ON DELETE CASCADE`). The apps let it go when they hear the message was deleted.

### Items and their bounds

An author's waiting rows in one pool are one item, which takes turns with the others. Its rows post in the order sent.

| Bound | Value | Past it |
| --- | --- | --- |
| An item | 50 rows (`MAX_ITEM_MESSAGES`), or 2,000 mana in all (`MAX_ITEM_MANA`), its first row whatever it costs | `429` `channelBusy`, `Retry-After` 10 seconds (`BUSY_RETRY`) |
| A pool's items | 1,000 (`MAX_ITEMS`) | The same |
| How long a row waits | 2 minutes (`MAX_WAIT`) | Dropped, its author told the channel was too busy |

## Draining

One job drains a pool while anything waits there: `drainManaQueue`, keyed by the channel, so one runner holds it anywhere in the fleet (`queue::drain_step`). It is saved with the first row to wait there.

- **Ticking:** a step ticks at 5 Hz (`TICK`) for ten seconds, then hands the job back with its pointer as its progress (`Outcome::Progress`), so its lease renews and any runner may take it up. A drainer takes a place of the `interactive` class like any job, so many busy channels at once wait their turns for places.
- **Each tick** first drops what has waited `MAX_WAIT`, a hundred at a time, telling each author. Then it reads each item's oldest row, sorts the items by when each began waiting, and spends by deficit round-robin from the pointer, for at most one trip round them:
  - When the pointer reaches an item, the item is given `QUANTUM` (100 mana) of credit.
  - The item posts its oldest row whenever its credit and the pool both cover that row's cost (`mana::pay_waiting`), taking the cost from both, and goes on while they do.
  - When its credit runs short, the pointer moves to the next item, whose quantum is yet to be given.
  - When the pool runs short, the tick ends, the pointer staying where it is with its quantum given.
  - An item whose last row posted is done, and its credit goes with it.
- **Fair over a trip:** every item gets the same credit a trip round, and whole rows are paid with whole mana, so shares never grow vanishingly small however long the queue. A row costing more than a quantum is paid over several turns.
- **Credit** is kept in the step's memory and saved on each item's oldest row as the step hands over, so a runner taking the job up carries on from it.
- **Posting a row** is posting as its kind would be (`held::post_waiting`): a message or a command by `message::post`, an edit by `update_message`, a poll by `create_poll`, each taking the row in its own transaction; an echo by `echo_reply`, its row going after, since echoing twice makes one echo. Permissions, blocks, and plugins decide it then; one that can no longer be posted is dropped, its author told why, and one that fails for a reason that may pass is left to wait.
- **Finishing:** a drainer that finds nothing waiting deletes its own job, in one statement, only while nothing waits (`finish_if_empty`), so a row saved at that moment sees the job gone and saves another (`ensure_drainer`).
- **A channel that can no longer have a pool** (made something else, or gone) has what waits there posted without paying.
- **Capacity, not fairness, bounds a long queue:** a thousand items of 100 mana each is 100,000 mana, over half an hour at 50 a second, so most are dropped at `MAX_WAIT`, evenly, since each waits as many trips.

## The bar

- **Telling viewers:** a spend publishes the pool's level as a relay on core NATS `aspen.mana` (`live::changed`), routed exactly as typing is (`event_feed::relayed_events`): to connections viewing the channel (`viewing`) that may read it, as an `ephemeral` frame, never replayed. The frame (`EphemeralEvent::Mana`) holds `channelId`, `level` (thousandths, below zero in debt), `max`, `recharge`, and `at`.
- **Twice a second at most:** the first spend in each half second (`live::EVERY`) sets `mana:{channel}:told` and is told; a server whose spend fell inside the window tells the level as it is when the window ends, so the last spend of a burst always reaches viewers.
- **Starting to view:** each event stream connection's task (`live::Viewer`) tells it the level of each channel it starts to view whose pool is below full and that it may read, so a bar already showing appears at once.
- **In the app:** the store keeps the latest level of up to 32 channels (`RecordStore.mana`), and the bar works out the level between frames from `at` and `recharge` at the server's ticks (`manaAt`). It shows once the level falls below 60% of `max` (`MANA_SHOW_BELOW`), and goes once the pool is full. It lies along the top of the typing line, so the conversation never moves for it.
- **A lost connection** forgets every level; viewing a channel anew is told it again.

## Settings

| Setting | Where | Default | Who changes it |
| --- | --- | --- | --- |
| `mana_max_recharge` | Deployment settings | 50 a second | Manage deployment settings |
| `mana_max` | Deployment settings | 20,000 | Manage deployment settings |
| `mana_attachment_cost` | Deployment settings | 30 | Manage deployment settings |
| `mana_recharge` | The channel (`manaRecharge`) | The deployment's `mana_max_recharge` | Manage channels |
| `mana_max` | The channel (`manaMax`) | The deployment's `mana_max` | Manage channels |

- **The deployment's are ceilings:** `POST /channels` and `PATCH /channels/{channel}` refuse a channel's above them, or below 1, with `validation` and a message naming the limit (`mana::check_channel_settings`).
- **Clamped where used:** a channel set above a deployment setting later lowered is held to it at once (`PoolSettings::of`).
- **`null` follows the deployment's**, as it changes.
- **The deployment's are public:** `GET /deployment` gives them as `mana`, for the channel's settings to show.
- **The deployment's** must be at least 1 (`manaSettingZero`).

## Rate limits

What one person may post (`rate_limits.toml`): messages and commands 50 per 5 seconds in a channel (bursts of 100) and 600 a minute (bursts of 200); thread replies 200 a minute (bursts of 100); edits 300 a minute (bursts of 100). A whole channel's flood has no limit here: its mana is that. Bots have ten times a person's (`bot_multiplier`).

## When Valkey is down

Mana cannot be counted, so everything posts as if every pool were full: it fails open, and the server logs it twice a minute at most. What already waits is paid for as it is posted, and so posts too.

## When access is given or taken away

1. **Who can observe it:** a pool's level, the connections viewing the channel that may read it (as typing, routed by the channel's governing permissions); a waiting row, only its author (user-scoped events, `GET /users/@me/held-messages`); a channel's settings, whoever reads the channel's record.
2. **What decides it:** reading the channel for the level, checked by the event feed's routing and, for the first frame, by `channel_access` (`live::for_viewer`); authorship for waiting rows; Manage channels for a channel's settings; Bypass channel mana, read by `channel_access` as each gate decides.
3. **When the deciding permission is lost:**
   - A waiting row is posted by the same code that posts it at once, which checks everything then: its author losing the right to post there (a role or override changed, a role taken or deleted, a move, a category deleted, removed, banned, leaving, a ban from the deployment, the account deleted, the channel deleted) drops it with `heldMessageFailed`, or the row goes with its author, channel, or target.
   - Losing Bypass affects only what is sent after.
   - The level stops reaching a connection as it loses the channel, as typing does.
4. **When it is gained:** a connection that starts viewing a busy channel is told its level. A new holder of Bypass goes ahead with their next message.
5. **Every path announces it:** spends publish the level; waiting rows are announced as posted, dropped, or cancelled; a channel's settings go out as `ChannelEvent::Update`, the deployment's through the settings' own announcement.
6. **Inside the transaction:** a waiting row and its drainer's job are saved in one transaction; posting a waiting row takes it in the posting's transaction. Spending is in Valkey, outside every transaction.

`scripts/check_permissions.py` checks that a member removed while their message waits has it dropped, and that a member who loses the channel stops hearing its level.

## Testing

- **Unit tests:** glyph counting and costs, settings and their ceilings (`app::mana`); the app's recharge arithmetic, store, and held edits (`client/packages/protocol/test/mana.test.ts`, `sync.test.ts`).
- **The `raid` benchmark scenario** (`aspen-bench`, see [Benchmarking](../benchmarking.md)) has a tenth of a 2,000-member community flood one channel, measuring how long everyone else's messages there wait and how many of either side's are held, refused, or dropped.
- **`scripts/check_mana.py`** drives a stack through it: spending and waiting, Bypass, draining in order and fairly, the bar's frames, edits (waiting, made, and changing nothing), polls, echoes, the two-minute limit, cancelling, and the bounds.
