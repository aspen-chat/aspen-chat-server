# Sending mail

## Without `[email]`

The deployment sends no mail:

- Accounts have no email address.
- Nobody can reset a forgotten password by email.
- The email [deployment settings](deployment-settings.md) cannot be turned on.

## With `[email]`

- People may give an address at registration or in their account settings.
- The deployment sends verification codes, password reset codes, the daily digest people ask
  for, and the newsletter, when its administrators run one.
- Mail waits in a queue in the database, password resets first.
- Every API server whose `send` is on sends it: by default all of them, so the SMTP server must
  accept connections from each.
- Links in mail, unsubscribing included, go to `public_url`.

## `[email]`

| Setting | Default | |
| --- | --- | --- |
| `send` | `true` | Whether this server sends mail and makes daily digests. At least one server of the deployment must send, or mail waits until one does. |
| `smtp_url` | required where `send` is on | The SMTP server mail is handed to. See [`smtp_url`](#smtp_url). |
| `from` | required | Who mail comes from, such as `Example Chat <noreply@chat.example.org>`. Its domain should publish SPF and DKIM records for your SMTP server, or mail lands in spam. |
| `max_per_second` | none | The most mail the whole deployment hands to the SMTP server in a second, however many servers send. See [Sending rate](#sending-rate). |
| `[email.tls]` | | Authorities to trust and a client certificate, for an `smtp_url` that always uses TLS. See [`[email.tls]`](#emailtls). |

### Sending from a few servers

A large deployment can turn `send` off on most servers, and leave sending, the SMTP credentials,
and making digests to a few.

- Those few may be private workers that serve nothing; see [Private workers](../installing/4-api-server.md#private-workers).
- Those few must reach the SMTP server.
- The rest need only `from`.
- A server with `send` off still takes addresses and queues mail. It tells the senders at once,
  over NATS, when someone waits for mail.

### `smtp_url`

| Form | Use |
| --- | --- |
| `smtps://user:password@smtp.example.org` | TLS from the start, port 465. |
| `smtp://user:password@smtp.example.org?tls=required` | STARTTLS, port 587. |
| `smtp://localhost:1025` | A development mail catcher, such as the `mailpit` service in `docker-compose.yaml` (its inbox is at http://localhost:8025). |

- Percent-encode characters in the user and password that a URL reserves.
- **The server refuses to start with a user or password in an `smtp://` address without
  `?tls=required`, unless the host is this machine.** `tls=opportunistic` is not enough.
- Any provider that takes SMTP works: Amazon SES, Postmark, Mailgun, your own Postfix.

### `[email.tls]`

For an `smtp_url` that always uses TLS (`smtps://` or `?tls=required`):

| Key | |
| --- | --- |
| `ca_file` | PEM authorities to trust besides the system's. |
| `cert_file`, `key_file` | A PEM client certificate and its key. Give both or neither. |

### Sending rate

- `max_per_second` is counted in Valkey, across every sending server.
- Set it under your provider's sending quota. Amazon SES starts accounts at 14 a second.
- A second's worth may go back to back.
- Left out, each sending server sends up to eight at once.

## Failed mail

- Mail the SMTP server refuses for good (an address that does not exist) is dropped and logged.
- Mail it cannot take now is tried again, waiting longer each time, for about a day.
- The metrics `aspen_emails_sent_total` and `aspen_emails_failed_total` count both.
