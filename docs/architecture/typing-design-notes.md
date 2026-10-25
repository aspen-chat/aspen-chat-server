# Typing: design notes

Why [Typing](typing.md) works as it does.

## Transport

- Typing is published on core NATS (`aspen.typing`), outside `aspen.events.>`. The JetStream stream
  then neither keeps nor replays it, and it travels over the connection each server already holds.
- It is delivered as an `ephemeral` frame, skipped when a connection's queue is full rather than
  dropping the connection. A lost word is said again at the next refresh or runs out, so nothing
  depends on any one frame arriving.

## Ending

- A connection's task says `typing: false` for every channel it showed when the connection ends, so
  a lost connection stops showing as soon as the server notices it. A server that stops altogether
  relies on the expiry instead.
- The client forgets everyone shown when its connection drops, since nobody can then tell it they
  stopped.

## Budget

- Each frame a client sends costs the server a check (a pooled database connection and a few reads)
  and each word published costs every API server a routing pass, so frames are budgeted per
  connection, by checks and publishes rather than by frames. A client alternating `typing` and
  `stoppedTyping`, or naming channels it cannot post in, gets no more work out of the server than
  one typing steadily.
- What a check found is remembered through stops and refusals, since a budget that forgot a channel
  at a stop or a refusal would let the next frame check it again at once.
- The check locks no row: holding the channel, as posting does, is for a transaction that writes
  after it, and a lock taken for every word would let typing frames write to the database.
- A stop goes only to an audience read in the last two seconds, or read again for it. One left to
  run out by the expiry costs a few seconds of a typing indicator; one told to a stale audience
  tells someone who just lost the channel that the typist was there.

## Blocks

- A typist's words never reach anyone they block. A block keeps the blocked from knowing when the
  blocker is about, as it does for presence.
- The word names only the blocked people the audience holds (`typing::blocked_in`), so a word
  repeated every few seconds names only those it could reach.

## Viewing

- A word reaches only connections that said they have the channel open (`viewing`). Typing is shown
  only beside a message box, so it reaches only those who could see it rather than every member of a
  busy community.

## Federation

- Nothing passes between deployments for typing. A user signed in abroad is a user there like any
  other, with an event stream of their own, so the same mechanism serves them.
