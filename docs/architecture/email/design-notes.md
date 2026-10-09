# Email: design notes

The reasons behind the [email](index.md) design.

## Addresses

### Addresses are not unique

- **Two accounts may share an address.** A unique index would tell anyone, at registration, whether an address is in use. See [Addresses](addresses.md).

### Changing an address needs reverification

- **Giving, changing, or removing an address is a security change.** A verified address can reset the password, so the change asks for a recently verified sign-in. See [Addresses](addresses.md#giving-changing-or-removing-an-address).

### Codes are kept as HMACs

- **Verification and reset codes are stored as their HMAC under a server secret.** Whoever reads Valkey cannot recover a code by trying every one. See [Verification](verification.md#codes).

## Password reset

- **Step 2 answers `204` whether or not the address matches.** A reset then tells nobody whether an address is the account's. See [Password reset](password-reset.md).
- **The per-account cap of five codes an hour is read before the address is compared.** Nobody can fill an inbox by starting resets from many addresses, and the refusal still tells nothing of the address.
- **The first step is limited by everyone's starts together.** A crowd of addresses cannot fill Valkey with resets.
- **Completing a reset removes the last week's second factors and recovery codes.** Whoever took over an account can add a factor of their own, and with its first one receive its recovery codes. The reset takes those away from them.
- **Older factors stay.** Signing in still asks for one of the owner's.
- **The reset does not make and mail new recovery codes.** Codes made by the reset and mailed would hand a second factor to whoever holds the mailbox, which is all a reset proves. The notice asks the owner to sign in and make new ones instead.
- **Bots keep their tokens.** A token is reissued only by its owner with a freshly verified sign-in.
- **Step 1 shows that a username exists.** Registration's `usernameTaken` does already, so this reveals nothing new. The address stays behind its mask.

## The outbox

- **Mail is queued in the causing transaction rather than sent during the request.** It goes exactly when what caused it commits, survives a restart, and keeps requests from waiting on SMTP.
- **`send` can be off on some servers.** A large deployment can keep the SMTP credentials and the work of sending and making digests on servers chosen for it, while a small one sends from every server.
- **Mail is a job of its own, classed by who waits for it.** A reset or a code is interactive, since someone waits at a screen; digests and newsletters are bulk. So a million newsletters never hold up a reset.
- **Waited-for mail wakes every job runner, plus a one-second look.** The wake makes waited-for mail prompt on every server; the look bounds what a lost wake-up costs.
- **One Valkey GCRA bucket for the sending rate.** The deployment keeps to its provider's quota however many servers send.
- **Mail that will not be sent is given up, not kept.** Once its attempts are spent, or the SMTP server refuses it for good, nothing more is done with it.
- **A Valkey failure lets mail through.** This matches the API's rate limits.

## The digest

- **Each account's due time is spread over its hour by its id.** Everyone who chose 8:00 in one zone is not due at once. See [The daily digest](digest.md#scheduling).
- **Each digest is made in a transaction of its own, with its row locked.** So each is made once, and one that fails is logged and skips its day rather than holding up the rest.
- **A digest's lower bounds are one message id (`aspen_uuid_floor`).** So each place is read through `message (channel, id)` from there on, never through its history.
- **A digest judges visibility when it is made.** It is a snapshot, so access lost after it is made does not change it.

## The newsletter

- **Mail is queued a thousand subscribers at a time, by a job saved beside the post.** A newsletter to a million subscribers is never one transaction. See [The newsletter](newsletter.md).
- **The unsubscribe `GET` only shows a confirmation page.** Mail scanners follow links, and must not unsubscribe anyone.
