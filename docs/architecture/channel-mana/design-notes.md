# Channel mana: design notes

Why channel mana works as it does. The how is in the [main page](index.md).

## The budget

- **A channel's budget, not only each person's.** A raid of many accounts, each within its own
  limits, still floods a channel. Bounding the channel bounds the crowd.
- **Glyphs, not messages.** A wall of text costs readers more than a word; counting messages would
  treat them alike.
- **Grapheme clusters, in every script alike.** What a reader sees as one character costs one, so
  Chinese, accented, and emoji text cost what English does. Counting code points or bytes would
  charge some scripts several times more. There is one path for every script; counting is cheap
  enough (a full 10,000-character message in about 0.15 ms) to need none faster for ASCII.
- **Past ten scalars a cluster costs by its length.** A grapheme cluster has no length limit: a ZWJ
  chain of a thousand emoji, or one letter under thousands of combining marks, is one cluster, and
  counted as one would let a full message cost a single glyph. Ten scalars hold the longest
  recommended emoji sequence and far more than any script's letter with its marks, so charging each
  scalar past ten leaves real text at its cost while stacked text pays nearly what it holds.
  Refusing long clusters instead would have to draw a line no script crosses, which is less sure
  than charging for them.
- **Tokens cost one.** A tag or a custom emoji is one thing to a reader, however long its stored
  form.
- **Counted on the text as sent.** The gates decide before the posting runs the channel's plugins,
  so a plugin that lengthens a message does not make its author pay for it.
- **Debt, capped at half the pool.** A message larger than everything a full pool holds still posts,
  or it could never post. Its cost becomes debt, so the next messages wait as they would after any
  large one, and the cap keeps one huge message (or a moderator's long post) from stalling a channel
  for minutes.

## Waiting rather than refusing

- **A message waits its turn rather than failing.** Under pressure, what someone wrote arrives late
  instead of being lost.
- **Waiting rows are held messages.** To their author they are one thing: accepted, not yet seen.
  The app already lists held messages and tells of their posting and dropping, so a second list
  would show the same thing twice.
- **What waits is bounded.** One item per person, of 50 rows or 2,000 mana; 1,000 items a pool; two
  minutes each. Beyond that, sending is refused with a retry time: a queue that drained for hours
  would post replies long after the conversation had moved on.
- **Edits that lengthen wait.** Letting them skip the queue would let anyone post through edits.
  Edits that shorten or reword cost nothing and apply at once, so a typo can always be fixed and
  something said taken back.
- **An echo waits apart from its reply.** The reply belongs to the thread, which can pay; holding it
  back for the parent's sake would hold the thread back too.

## Who goes first

- **Not first come, first served.** With a single queue in arrival order, raiders need not get their
  spam through: they only need to drain the pool, and every regular's message waits behind theirs.
- **Not equal shares either.** Turns shared equally by author cannot tell a raid's accounts from
  regulars: with 145 raiders each person had a 146th of the channel, and in the `raid` benchmark
  everyone else's messages there waited a median of half a minute, two minutes at worst, and two in
  five were dropped.
- **Rank, then tenure.** A raid is many new accounts holding no roles. Serving the higher ranked
  first, then those who have belonged longer, puts every regular ahead of it, and it delays only
  itself.
- **Tenure in whole days, then a keyed hash.** A raid's accounts join seconds apart; in exact join
  order the first of them would take everything the pool earns. Counted in days they are equals, and
  among equals the order is an HMAC of the channel, the author, and their username under a secret
  only the servers hold.
- **Why not random.** A fresh draw for each message shares the channel between equals in
  expectation, but the drawn author has to be remembered until their row is paid for, or the cheap
  messages of equals would always post first and a costly one never; that memory is state to carry
  between ticks and servers. A fixed order needs none: every server and every tick agree on it.
- **Why not a plain hash.** A hash of the username anyone can compute lets raiders grind names
  offline until theirs sort first among equals, and they choose their names. Keyed with the
  deployment's secret, no one outside the servers can tell which name wins, so there is nothing to
  grind. The channel is in it so no account sorts first everywhere; the user id, so two accounts
  never tie.
- **Thirds, not the whole pool to the first.** Under strict priority the first author's row would
  have to be paid in full before anyone below posted: one costly row from a moderator or regular
  would hold back everyone else, and a long first row could stall a channel until it was dropped.
  Splitting what the pool earns among the first three keeps rank decisive (the first is credited at
  least a third of everything, and all of it when alone) while the second and third keep moving, so
  one row never blocks a channel. Three, because one place is strict priority again, while many
  places dilute the head of the queue towards equal shares, which a raid of many accounts wins; with
  three, regulars waiting fill every place, and a raid gets at most two while one regular waits.
- **Credit returns when it is not needed.** An author pushed out of the first three, or with nothing
  left to post, gives back what they held, so mana is never parked with someone who will not spend
  it.
- **Credit lives beside the pool.** It is the pool's mana, reserved, so it must change in the same
  atomic step as the level: in Valkey, by script, with the pool. The pool's own script counts it, so
  a spend by someone with Bypass cannot spend what is credited, and the bar shows it as mana the
  channel has.
- **No one displaces a higher rank or an equal.** A full queue drops the last author waiting only
  for someone strictly above them, so a raid larger than the queue cannot keep regulars out, and no
  one can push out someone of higher rank.
- **An author's waiting rows are one item.** Someone who sends dozens of short lines has one place
  in the queue, as someone who wrote one long message does.

## Moderators and bots

- **Bypass still spends.** A moderator's message goes ahead, but the channel's mana still shows
  everything said.
- **Bots get nothing of their own.** A chatty bot spends what everyone shares; one that must post
  freely is given Bypass through its role, a choice the community makes.
- **No Bypass in DMs.** A group DM has no moderators, and Bypass is only ever a community's choice.

## Where its state lives

- **The pool in Valkey; what was promised in PostgreSQL.** The level changes with every message on
  every server, so it needs an atomic step at memory speed, and losing it costs nothing worse than a
  full pool. A `202` promises a message will be posted or its author told otherwise, so waiting rows
  must survive a restart, and posting one takes its row in the same transaction, so it posts once.
- **Lazy pools.** Nothing ticks while a channel is quiet: whoever touches a pool adds the ticks
  since. Only a pool with something waiting has a drainer, the one thing in the fleet that ticks
  this fast.
- **Thousandths of mana.** Every rate a second divides into 5 Hz ticks without fractions.
- **One drainer per pool, as a job.** The job's key gives one runner per pool across the fleet, its
  lease moves it when a runner goes, and its progress carries the pointer, with no leader election
  of its own.
- **Fails open.** Mana guards readers from floods, not secrets, so an outage of Valkey lets
  everything through rather than silencing every channel.

## The bar

- **Hidden until it matters.** A quiet channel never shows it; below 60% it appears, and it stays
  until full so it does not flicker around the threshold.
- **Levels and a rate, animated by the app.** The recharge is predictable, so a frame twice a second
  while spending is enough, and none while a pool is full.
- **Over the typing line.** It takes no room of its own, so the conversation never moves when it
  comes or goes.
