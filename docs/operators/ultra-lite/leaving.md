# Leaving Ultra Lite

A deployment that outgrows one machine moves to the full one (see
[Installing](../installing/index.md)) with its accounts, messages, files, sign-ins, and address.
People stay signed in; their apps reload what they show. The move is one way: nothing moves back to
Ultra Lite.

| What moves                 | How                                                                                                     |
| -------------------------- | ------------------------------------------------------------------------------------------------------- |
| The database               | A dump, restored into the new deployment's PostgreSQL (18 or later). Its keys and secrets come with it. |
| Uploaded files             | Uploaded into the new deployment's object storage, each with the type it is served with.                |
| The address (`public_url`) | Kept exactly: passkeys and other deployments know the deployment by it.                                 |
| The certificate            | Not moved: the new deployment gets its own.                                                             |
| The voice server           | Not moved: Ultra Lite's is removed, and the new deployment registers its own.                           |

On AWS, `deploy/aws/migrate.py` does every step below; see
[Moving Ultra Lite here](../../../deploy/aws/README.md#moving-ultra-lite-here). Anywhere else,
follow them.

## 1. Export

The deployment is down from here until the new one serves.

```
sudo systemctl stop aspen-ul
sudo install -d -o aspen-ul -g aspen-ul -m 700 /var/backups/aspen-ul
sudo aspen-ul export /var/backups/aspen-ul/export
```

The export holds `database.dump`, `media/` (the files, unless they are already in object storage),
and `export.json`, which names the address (`publicUrl`) and the voice server to remove. **It is as
secret as the deployment**: copy it to the new deployment's machines with `rsync -a`, keeping it
readable by you alone.

## 2. Restore the database

Into an empty database of the new deployment's PostgreSQL, as a user that may create extensions:

```
pg_restore --no-owner --no-privileges --exit-on-error --single-transaction \
  --dbname='<the database_url>' export/database.dump
aspen-migrate up --database-url '<the database_url>'
```

`aspen-migrate` applies whatever the new release has that the export lacked.

## 3. Configure the new deployment

Set `public_url` in `aspen.toml` to `export.json`'s `publicUrl`, exactly. Give it its object storage
(`[media.s3]`) and the rest as [Installing](../installing/index.md) says.

## 4. Upload the files

Where the API server runs, with its `aspen.toml`:

```
aspen-chat-server storage import export
```

It uploads every file into the bucket with its type, checking each by its size once there. Run it
again if it stops; it uploads only what is not there yet. A deployment that kept its files in object
storage already skips this step: point `[media.s3]` at that bucket, or copy its objects into the new
bucket with `aws s3 sync s3://<old> s3://<new>` (a copy between buckets keeps each file's type).

## 5. Replace the voice server

```
aspen-chat-server voice-servers remove aspen-ul
```

Then register the new deployment's voice servers
([Voice servers](../installing/6-voice-servers.md)).

## 6. Start it, and move the names

Start the servers, then point both names (the deployment's and the files', if the new storage's read
path has another) at the new deployment. When it answers at its address, keep Ultra Lite from
starting again (`sudo systemctl disable aspen-ul`), and keep its data directory and the export until
the new deployment has run a while: they are the way back.

## Moving the files to object storage, staying on Ultra Lite

A deployment whose files outgrow its disk keeps them in S3-compatible storage instead:

1. Stop it: `sudo systemctl stop aspen-ul`.
2. Add [`[server.media.s3]`](configuration.md#server-everything-else) to `aspen-ul.toml`. The
   storage must serve the files at its read path (`public_base_url`) and allow uploads from the
   deployment's origin by CORS ([S3](../configuration/media.md)).
3. Move them: `sudo aspen-ul move-to-s3`. It uploads every file with its type, checks each, and then
   moves `media/` aside to `media.moved-<time>/`. Run it again if it stops.
4. Start it: `sudo systemctl start aspen-ul`. Once it serves the files, remove
   `media.moved-<time>/`.

Ultra Lite refuses to start with `[server.media.s3]` given while `media/` still holds files. Backups
no longer hold the files once they are in object storage.
