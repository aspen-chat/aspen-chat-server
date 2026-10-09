# The daily digest

An account that turns on `digest` at a verified address receives mail once a day, at `digestHour` o'clock in `digestTimeZone`. It tells what arrived for the account since the digest before (`app::email::digest`).

## Scheduling

| Column | Meaning |
| --- | --- |
| `user_email.digest_next_at` | When the next digest is due (`digest::next_due`). |
| `user_email.digest_since` | Where the next digest starts. Set when the digest is turned on, so the first covers only what arrived after. |

`digest::next_due` gives:

- the next time that hour comes in the zone;
- the hour after, when a clock change skips it;
- at a fixed point of the hour drawn from the account's id (`digest::spread`). So everyone who chose 8:00 in one zone is spread over the hour rather than due at once.

## Making digests

1. A recurring job, `makeDigests`, runs every minute at the bulk class on a server that sends mail (see [Jobs](../jobs/index.md)).
2. Each step takes twenty due digests, the longest due first. Each is made in a transaction of its own with its row locked, so each is made once.
3. It makes each digest (below) and queues it in the [outbox](outbox.md). One that fails is logged and skips its day rather than holding up the rest.
4. Whether it is sent or not, the next is scheduled and `digest_since` moves on.

A digest with nothing in it is not sent, and a banned account's is skipped.

## What a digest covers

- The channels the account may view in its communities (`Visibility`, as it stands when the digest is made).
- Its DMs and group DMs.
- Never threads.

It leaves out what never makes a channel unread (`app::read_state`):

- the account's own messages;
- messages of anyone it blocked;
- messages at or before its read position;
- channels and DMs it has muted.

Its lower bounds (the digest before, the moment it joined, its read position) are one message id (`aspen_uuid_floor`). So each place is read through the index `message (channel, id)` from there on, never through its history.

## What a digest holds

| Limit | Value |
| --- | --- |
| Places told of | The twenty where something arrived most recently |
| Messages per place | The five earliest, plus how many more it holds |
| Message length | 300 characters of plain text |

- Text has tags and custom emoji by name (`Names::excerpt`).
- Each place links to itself in the web client (`public_url`).
- The digest says how many more places hold something.
