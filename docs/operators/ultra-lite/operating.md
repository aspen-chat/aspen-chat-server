# Running Ultra Lite

## Commands

Every command of the full server's (`admin`, `invites`, `settings`, `plugins`, and the rest; see
[Deployment settings](../configuration/deployment-settings.md),
[The first administrator](../installing/7-first-administrator.md), and [Plugins](../plugins.md)) is
a command of `aspen-ul`'s:

```
sudo aspen-ul settings show
sudo aspen-ul invites create --uses 5
```

Run as root, a command runs as the user `aspen-ul`. While the deployment runs, what a command
changes reaches everyone connected at once. While it is stopped, the command starts the database
itself.

`sudo aspen-ul doctor` checks the machine, the deployment, DNS, the router, and what the internet
sees, and says what to fix. Run it whenever something does not work.

## Logs

- The deployment's: `journalctl -u aspen-ul`.
- PostgreSQL's: `/var/lib/aspen-ul/logs/postgres.log`.

## Backups

```
sudo install -d -o aspen-ul -g aspen-ul -m 700 /var/backups/aspen-ul
sudo aspen-ul backup /var/backups/aspen-ul/$(date +%F)
```

A backup is a directory of its own holding the database, every uploaded file, `aspen-ul.toml`, and
the certificate. It is made while the deployment runs, with nothing interrupted. The directory must
be one the user `aspen-ul` can write, and empty or absent.

**A backup is as secret as the deployment.** It holds the deployment's federation key, every
session, and every password hash: anyone holding one can sign as your deployment. Keep backups
readable by you alone, copy them to another machine, and encrypt them there. When files are kept in
S3, a backup does not hold them; keep them as the storage allows.

Run backups on a schedule (a systemd timer or cron) and try restoring one now and then.

## Restoring

```
sudo systemctl stop aspen-ul
sudo aspen-ul restore /var/backups/aspen-ul/2026-11-22
sudo systemctl start aspen-ul
```

What the data directory held is moved into `/var/lib/aspen-ul/replaced-<time>/`, not deleted; remove
it once the restored deployment works. Restoring on another machine is the same, after
`aspen-ul install` there. People stay signed in, and their apps reload what they show.

## Upgrading

Install the new `aspen-ul` the way it was first installed:

```
sudo ./aspen-ul install
```

It replaces the program and restarts the deployment, which is down for a few seconds. A start that
changes the database's tables first saves a copy of it in `/var/lib/aspen-ul/backups/` (the newest
three are kept), so an upgrade that goes wrong can be undone: install the previous `aspen-ul` and
restore that copy with its PostgreSQL's `pg_restore`.

An `aspen-ul` that carries a newer PostgreSQL upgrades the database to it on its first start,
keeping the old one until the new one has started. That needs disk for a second copy of the
database. Upgrade from each release to the next: the PostgreSQL of the release before must still be
in `/var/lib/aspen-ul/runtime/`, where it is kept for this.

## What it keeps

Everything is in `/var/lib/aspen-ul`:

| Path            | What                                                                                  |
| --------------- | ------------------------------------------------------------------------------------- |
| `aspen-ul.toml` | The configuration.                                                                    |
| `postgres/`     | The database. Never copy it while the deployment runs; use `aspen-ul backup`.         |
| `media/`        | Uploaded files, icons, and avatars.                                                   |
| `acme/`         | The certificate, its key, and the Let's Encrypt account.                              |
| `backups/`      | Copies of the database saved before an upgrade changed it.                            |
| `runtime/`      | The PostgreSQL and web client unpacked from the program, this one and the one before. |
| `logs/`, `run/` | PostgreSQL's log, and its socket and the commands' socket.                            |
