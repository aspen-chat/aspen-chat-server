# The outbox

Nothing is sent while a request waits. Whatever causes mail queues it as a job of its own, `sendEmail`, and servers that send mail run those jobs (see [Jobs](../jobs/index.md)).

## Queueing

`outbox::queue` saves the job in the causing work's own transaction. It holds:

- the account;
- the address when the mail names one (a code for the address it verifies, a notice to the address an account left), or none for the account's verified address as it is when sent;
- the mail as JSON (`outbox::Mail`).

So mail goes exactly when what caused it commits, and survives a restart.

An account deleted, or an address removed, takes the mail waiting for it with it (`outbox::forget_queued`, through the index `job_send_email_user`).

## Classes

Each piece's class orders it among every job of the deployment (`Mail::class`):

| Class | Mail |
| --- | --- |
| Interactive | A password reset, a verification code: someone waits for them at a screen |
| Normal | Notices, a newsletter test |
| Bulk | Digests, newsletters |

So a million newsletters never hold up a reset.

## Which servers send

- Servers whose `[email]` has `send` on (the default) send it (`outbox::send_step`).
- Those with it off only queue, and need no SMTP credentials. They also make no digests.

## Sending a piece

1. Where `[email] max_per_second` is set, the sender takes the piece from the rate bucket (below).
2. It checks the piece's recipient (`outbox::recipient`, below).
3. It hands the piece to the SMTP server.

## Waking job runners

- A server that queues mail someone waits for (a reset, a code, a newsletter test) wakes every job runner once it commits (`app::email::wake`, `jobs::wake`).
- Otherwise each runner looks every second.

## The sending rate

Where `[email] max_per_second` is set:

- Every sender takes each piece from one GCRA bucket in Valkey, `email:send-rate`, before handing it to the SMTP server. It uses `app::rate_limit::take`, the script the API's limits use.
- So the deployment keeps to its provider's quota however many servers send.
- A Valkey failure lets mail through unthrottled, as the API's limits do, and is logged.

## Recipient checks

`outbox::recipient` checks each piece as it is sent:

- Nothing goes to a deleted account.
- A digest or newsletter goes only to an account still verified and still subscribed.
- A code goes only to the address the account still holds.

## Results

| Result | What happens |
| --- | --- |
| Sent | The piece is done. |
| Refused for good by the SMTP server | The piece is given up and logged. |
| SMTP server cannot take it now | It waits a minute, doubling each time, for eleven attempts (about a day), and is then given up. |

No mail is kept once it will not be sent. The metrics `aspen_emails_sent_total` and `aspen_emails_failed_total` count them.

[Design notes](design-notes.md#the-outbox)
