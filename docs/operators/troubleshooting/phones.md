# Phones

## Phones are not woken

The app registers each phone when it signs in. The server then calls the phone's relay over
HTTPS.

Check that:

1. `[push] enabled` is on.
2. The server can reach the relay the phones' app names, over HTTPS. The log warns of any push
   that failed, by its subscription, and names the relay's address at `debug`
   (`ASPEN_LOG=aspen_app::push=debug`).

### Relay refusals

The log warns of every push a relay refuses.

`quotaExhausted` means the deployment has sent more pushes this month than its tier with that
relay allows. Pushes resume next month, or when its tier is raised.

### Unanswered relays

A relay that, five times in a row for one phone, does not answer within three seconds or cannot
be connected to, has that phone skipped for an hour.

`aspen_pushes_total` counts pushes by outcome, among them `unanswered`, `suspended`, and
`dropped`.

### Pushes that are not sent

Nobody is woken for a message:

- while they are using Aspen on another device,
- in a muted channel,
- from someone they blocked.
