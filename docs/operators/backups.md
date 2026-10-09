# Backups

Two things hold everything that cannot be made again: **the PostgreSQL database** and **the
object storage bucket**. Back up both, on a schedule, somewhere other than the machines they run
on, and try restoring now and then.

## The database

Every account, community, channel, message, reaction, poll, role, invite, setting, and session
is in PostgreSQL, as are the deployment's own settings (its name, policies, and federation
gates), its registered voice servers, and its federation key. Any PostgreSQL backup works: a
nightly `pg_dump -Fc`, or continuous archiving (base backups and WAL) when losing a day of
messages is too much.

**The database's backups are as secret as its key.** The federation key is what other
deployments trust when this one vouches for its people, and it is stored in the database so
that every API server signs with the same one. Anyone holding a backup can sign as your
deployment. Encrypt backups, and limit who can read them. If one leaks, replace the key with
`aspen-chat-server federation rotate-key --compromised` and tell the deployments you federate
with at once (see [Federation](federation.md#keys)): each refuses everything signed as you, and
signs your people out, from when it notices the new key (within about an hour, or at once if its
administrators run `federation contact`) until its administrators accept the new key. The sessions and password hashes in it are
sensitive too: a leak means everyone should change their password. So are people's email
addresses, and the mail waiting to be sent, which can hold a password reset code for the few
seconds before it goes.

The database also holds the deployment's push key, which phones' relays know it by. A
deployment restored without it makes a new one, and phones then need to be opened once to be
woken again.

It also holds the key the verification and password reset codes waiting in Valkey are kept
under (`server_secret`), so that a copy of Valkey alone does not give the codes away. A
deployment restored without it makes a new one, and codes mailed before then stop working; the
people waiting on them ask for new ones.

And it holds the key join tokens are signed with (`server_secret`). A deployment restored
without it makes a new one; voice servers ask for it as they meet a token naming it, so joins go
on, and only tokens handed out just before the restore stop working.

## The object storage

Attachments, icons, avatars, and link preview images are objects in the `[media.s3]` bucket;
the database holds only their names. Back the bucket up with your storage's own tools
(replication, versioning, or copying it elsewhere with `rclone` or `aws s3 sync`). A restored
database whose bucket was lost shows every picture and file as missing, and the people who
posted them would have to post them again.

The bucket also holds, under `evidence/`, the files of deleted messages and of attachments taken
off their messages, kept for reviewing reports. They are deleted once the
deployment setting `evidence_retention_days` (365 by default; 0 keeps them for good) has passed,
unless a report about their message is open or closed within as long, and at once by
`aspen-chat-server attachments purge --message <id>` (or `--attachment <id>`), which deletes them
from the database and the bucket and writes the purge to the moderation log; a backup made before
a purge still holds what was purged, so restoring one brings it back until it is purged again.

## What needs no backup

- **NATS** holds only the last minute of events, voice servers' reports waiting to be applied,
  and the number of the last change to the deployment's settings. After a loss, every client
  notices the gap, reloads what it shows, and carries on, within a minute each voice server's
  next snapshot puts the record of its calls right, and each API server reads the settings
  afresh.
- **Valkey** holds rate limit counters, who is online, and sign-in steps in progress (tickets
  for a second factor, passkey ceremonies, email verification codes, password resets). After a
  loss, people halfway through signing in, verifying an address, or resetting a password start
  again; nobody is signed out.
- **The servers themselves** keep nothing: rebuild or redeploy them.
- **`aspen.toml` and `voice_server.toml`** are configuration, not data, but they hold secrets
  (the database password, the storage keys). Keep them with your other secrets,
  not in the same place as the backups.

## Restoring

1. Restore the database (`pg_restore`, or your archive's recovery).
2. Restore the bucket.
3. Run `aspen-migrate up`, in case the restored database is older than the servers.
4. Start the API servers with the same `public_url`. They use the key in the restored
   database, so other deployments still recognise yours.

Sessions in the backup still work; anyone who signed in after it was taken signs in again.
Everyone's apps reload what they show.

If the database is restored from a backup older than a key replacement
(`federation rotate-key`), the restored deployment signs with the old key, which the deployments
that followed the replacement no longer accept: replace the key again with `rotate-key
--compromised`, and ask their administrators to accept the new one.
