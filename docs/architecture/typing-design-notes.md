# Typing: design notes

Why [Typing](typing.md) works as it does.

## Transport

- Typing is published on core NATS (`aspen.typing`), outside `aspen.events.>`. The JetStream stream then neither keeps nor replays it, and it travels over the connection each server already holds.
- It is delivered as an `ephemeral` frame, skipped when a connection's queue is full rather than dropping the connection. A lost word is said again at the next refresh or runs out, so nothing depends on any one frame arriving.

## Ending

- A connection's task says `typing: false` for every channel it showed when the connection ends, so a lost connection stops showing as soon as the server notices it. A server that stops altogether relies on the expiry instead.
- The client forgets everyone shown when its connection drops, since nobody can then tell it they stopped.

## Blocks

- A typist's words never reach anyone they block. A block keeps the blocked from knowing when the blocker is about, as it does for presence.
- The word names only the blocked people the audience holds (`typing::blocked_in`), so a word repeated every few seconds names only those it could reach.

## Viewing

- A word reaches only connections that said they have the channel open (`viewing`). Typing is shown only beside a message box, so it reaches only those who could see it rather than every member of a busy community.

## Federation

- Nothing passes between deployments for typing. A user signed in abroad is a user there like any other, with an event stream of their own, so the same mechanism serves them.
