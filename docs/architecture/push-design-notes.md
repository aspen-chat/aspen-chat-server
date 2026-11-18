# Push: design notes

Why [Push](push.md) works as it does.

## Signing

- At most 1024 audience tokens are kept (`webpush::MAX_TOKENS`), since endpoints' origins are
  subscribers' to choose.

## Subscriptions

- At most ten per account, trimmed on each registration under a lock on the `user` row. A message
  then costs a bounded number of pushes per person, however often they sign in.
- One subscription per refresh token, cascading. Signing out, a password change, or a ban stops the
  phone with the sign-in.

## Dispatching

- One durable JetStream consumer (`aspen_push`) is shared by every API server, so each event is
  handled once.
- The dispatcher never waits for a push service. It queues each push as a task of its own with
  per-origin and per-server slots, so a slow or unreachable push service delays only its own pushes.
- Failure-count suspension lives in Valkey, so every server skips a failing subscription, not only
  the one that saw it fail.
- The dispatcher waits for the commit before reading, since events are published before their
  transaction commits. A message not there after `COMMIT_WAIT` was rolled back.

## Attention rather than activity

- Presence's `active` key would not do: it lasts ten minutes and belongs to the user, so it would
  keep a phone silent long after the app on it was put away, and while a desktop window sat
  unwatched. What decides a push is whether something is showing the message now, so a connection
  says so itself and says at once when it stops.
- Sixty seconds without input ends a connection's attention: someone reading without touching the
  screen is not woken for a minute, and a desktop left focused stops holding pushes back soon after.
- One member per connection, not one key per user, so a phone put away is woken even while the
  user's desktop is still focused but idle, once that one's minute passes.
- A message that arrives while its reader is attended elsewhere is not held and sent later unless
  read: holding every attended recipient's push for a read check is a timer per person on every
  message, which a message to a large community cannot afford.

## The badge

- A counted badge is kept in Valkey for five minutes (`BADGE_KEPT`) and adjusted as messages arrive.
  Counting it (`push::badge_of`) takes a few queries. A change no push follows (a role or a channel
  taken away) therefore reaches the badge only within those minutes. See
  [The badge](push.md#the-badge).
