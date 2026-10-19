# Channel mana: design notes

Why channel mana works as it does. The how is in the [main page](index.md).

## The budget

- **A channel's budget, not only each person's.** A raid of many accounts, each within its own limits, still floods a channel. Bounding the channel bounds the crowd.
- **Glyphs, not messages.** A wall of text costs readers more than a word; counting messages would treat them alike.
- **Grapheme clusters, in every script alike.** What a reader sees as one character costs one, so Chinese, accented, and emoji text cost what English does. Counting code points or bytes would charge some scripts several times more. There is one path for every script; counting is cheap enough (a full 10,000-character message in about 0.15 ms) to need none faster for ASCII.
- **Tokens cost one.** A tag or a custom emoji is one thing to a reader, however long its stored form.
- **Counted on the text as sent.** The gates decide before the posting runs the channel's plugins, so a plugin that lengthens a message does not make its author pay for it.
- **Debt, capped at half the pool.** A message larger than everything a full pool holds still posts, or it could never post. Its cost becomes debt, so the next messages wait as they would after any large one, and the cap keeps one huge message (or a moderator's long post) from stalling a channel for minutes.

## Waiting rather than refusing

- **A message waits its turn rather than failing.** Under pressure, what someone wrote arrives late instead of being lost.
- **Waiting rows are held messages.** To their author they are one thing: accepted, not yet seen. The app already lists held messages and tells of their posting and dropping, so a second list would show the same thing twice.
- **What waits is bounded.** One item per person, of 50 rows or 2,000 mana; 1,000 items a pool; two minutes each. Beyond that, sending is refused with a retry time: a queue that drained for hours would post replies long after the conversation had moved on.
- **Edits that lengthen wait.** Letting them skip the queue would let anyone post through edits. Edits that shorten or reword cost nothing and apply at once, so a typo can always be fixed and something said taken back.
- **An echo waits apart from its reply.** The reply belongs to the thread, which can pay; holding it back for the parent's sake would hold the thread back too.

## Fair sharing

- **Round-robin by author, not first come first served.** With a single queue in arrival order, raiders need not get their spam through: they only need to drain the pool, and every regular's message waits behind theirs. Turns by author mean a raid mostly delays itself.
- **An author's waiting rows are one item.** Someone who sends dozens of short lines takes no more turns than someone who wrote one long message.
- **Deficit round-robin, not equal shares of each tick.** Split a tick's mana a thousand ways and each share is a hundredth of a mana, which whole messages can never be paid with. Giving each item a quantum a turn, with a pointer kept across ticks, pays whole messages with whole mana however long the queue; fairness then holds over a trip round rather than within a tick.
- **A quantum of 100 mana:** about one ordinary message, so most turns post a message, and few are spent only visiting.

## Moderators and bots

- **Bypass still spends.** A moderator's message goes ahead, but the channel's mana still shows everything said.
- **Bots get nothing of their own.** A chatty bot spends what everyone shares; one that must post freely is given Bypass through its role, a choice the community makes.
- **No Bypass in DMs.** A group DM has no moderators, and Bypass is only ever a community's choice.

## Where its state lives

- **The pool in Valkey; what was promised in PostgreSQL.** The level changes with every message on every server, so it needs an atomic step at memory speed, and losing it costs nothing worse than a full pool. A `202` promises a message will be posted or its author told otherwise, so waiting rows must survive a restart, and posting one takes its row in the same transaction, so it posts once.
- **Lazy pools.** Nothing ticks while a channel is quiet: whoever touches a pool adds the ticks since. Only a pool with something waiting has a drainer, the one thing in the fleet that ticks this fast.
- **Thousandths of mana.** Every rate a second divides into 5 Hz ticks without fractions.
- **One drainer per pool, as a job.** The job's key gives one runner per pool across the fleet, its lease moves it when a runner goes, and its progress carries the pointer, with no leader election of its own.
- **Fails open.** Mana guards readers from floods, not secrets, so an outage of Valkey lets everything through rather than silencing every channel.

## The bar

- **Hidden until it matters.** A quiet channel never shows it; below 60% it appears, and it stays until full so it does not flicker around the threshold.
- **Levels and a rate, animated by the app.** The recharge is predictable, so a frame twice a second while spending is enough, and none while a pool is full.
- **Over the typing line.** It takes no room of its own, so the conversation never moves when it comes or goes.
