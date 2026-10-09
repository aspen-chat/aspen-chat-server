# Backups

Two things hold everything that cannot be made again:

- **the PostgreSQL database**,
- **the object storage bucket**.

Back up both, on a schedule, somewhere other than the machines they run on. Try restoring now and
then.

## The database

PostgreSQL holds every account, community, channel, message, reaction, poll, role, invite,
setting, and session. It also holds the deployment's own settings (its name, policies, and
federation gates), its registered voice servers, and its keys (below).

Any PostgreSQL backup works:

- a nightly `pg_dump -Fc`, or
- continuous archiving (base backups and WAL), when losing a day of messages is too much.

### Keep its backups secret

**The database's backups are as secret as its federation key.** Anyone holding a backup can sign
as your deployment. Encrypt backups, and limit who can read them.

The backups also hold:

- sessions and password hashes: a leak means everyone should change their password;
- people's email addresses;
- the mail waiting to be sent, which can hold a password reset code for the few seconds before
  it goes.

### If a backup leaks

1. Replace the key:

   ```
   aspen-chat-server federation rotate-key --compromised
   ```

2. Tell the deployments you federate with at once (see [Keys](federation/keys.md)).

Each of them refuses everything signed as you, and signs your people out, from when it notices
the new key until its administrators accept it. It notices within about an hour, or at once if
its administrators run `federation contact`.

### The keys in the database

| Key | What it does | Restored without it |
| --- | --- | --- |
| The federation key | What other deployments trust when this one vouches for its people. Every API server signs with it. | See [Restoring](#restoring). |
| The push key | What phones' relays know the deployment by. | A new one is made. Phones need to be opened once to be woken again. |
| The code key (a row of the `server_secret` table) | What the verification and password reset codes waiting in Valkey are kept under. | A new one is made. Codes mailed before then stop working, and the people waiting on them ask for new ones. |
| The join token key (another row of `server_secret`) | What join tokens are signed with. | A new one is made. Voice servers ask for it as they meet a token naming it, so joins go on. Only tokens handed out just before the restore stop working. |

## The object storage

Attachments, icons, avatars, and link preview images are objects in the `[media.s3]` bucket. The
database holds only their names.

Back the bucket up with your storage's own tools: replication, versioning, or copying it
elsewhere with `rclone` or `aws s3 sync`.

**A restored database whose bucket was lost shows every picture and file as missing.** The people
who posted them would have to post them again.

### Evidence

The bucket also holds, under `evidence/`, the files of deleted messages and of attachments taken
off their messages, kept for reviewing reports. They are deleted:

- once the deployment setting `evidence_retention_days` (365 by default; `0` keeps them for good)
  has passed, unless a report about their message is open, or was closed within as long;
- at once by a purge:

```
aspen-chat-server attachments purge --message <id>
aspen-chat-server attachments purge --attachment <id>
```

A purge deletes them from the database and the bucket, and writes the purge to the moderation log.

**A backup made before a purge still holds what was purged.** Restoring one brings it back until
it is purged again.

## What needs no backup

| Part | What it holds | After a loss |
| --- | --- | --- |
| NATS | The last minute of events, voice servers' reports waiting to be applied, and the number of the last change to the deployment's settings. | Every client notices the gap, reloads what it shows, and carries on. Within a minute each voice server's next snapshot puts the record of its calls right. Each API server reads the settings afresh. |
| Valkey | Rate limit counters, who is online, and sign-in steps in progress (tickets for a second factor, passkey ceremonies, email verification codes, password resets). | People halfway through signing in, verifying an address, or resetting a password start again. Nobody is signed out. |
| The servers themselves | Nothing. | Rebuild or redeploy them. |
| `aspen.toml` and `voice_server.toml` | Configuration, not data, but with secrets (the database password, the storage keys). | Keep them with your other secrets, not in the same place as the backups. |

## Restoring

1. Restore the database (`pg_restore`, or your archive's recovery).
2. Restore the bucket.
3. Run `aspen-migrate up`, in case the restored database is older than the servers.
4. Start the API servers with the same `public_url`. They use the key in the restored database,
   so other deployments still recognise yours.

Afterwards:

- Sessions in the backup still work. Anyone who signed in after it was taken signs in again.
- Everyone's apps reload what they show.

### Restoring from before a key replacement

If the database is restored from a backup older than a key replacement
(`federation rotate-key`), the restored deployment signs with the old key. The deployments that
followed the replacement no longer accept it.

1. Replace the key again with `rotate-key --compromised`.
2. Ask their administrators to accept the new one.
