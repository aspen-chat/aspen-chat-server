# Aspen on one machine

`aspen-box` runs a whole deployment on one Linux machine with Docker: a small server you rent, or a
small computer you own. It is for a community of tens of people. More voice servers can run on other
machines, each a box of its own.

**New and lightly tried.** `scripts/check_box.py` starts, upgrades, backs up, restores, and exports
it, with a voice server on a second machine and a database elsewhere, every machine's network stood
in for. It has not yet run on an ARM computer, on a real domain, or behind a home router.

## What it needs

|                                 |                                                                                                                                                                                   |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A machine                       | 64-bit Linux, 2 GiB of memory or more (4 is comfortable), and an SSD. Not an SD card: see [At home](#at-home).                                                                    |
| Docker                          | The system's, with Compose 2.20 or later. Not rootless Docker, whose containers nothing can reach.                                                                                |
| Python 3.9                      | Nothing else: the tool uses only what comes with it.                                                                                                                              |
| A domain                        | One name for the deployment, **for good** ([Choosing `public_url`](../../docs/operators/installing/3-configuring.md#choosing-public_url)), and the right to add records under it. |
| An address the internet reaches | The machine's own, or a router's that forwards to it.                                                                                                                             |

## Getting the tool

From a checkout of the repository, it is `deploy/box/aspen-box`. Without one, every release's `api`
image holds it:

```
mkdir ~/aspen-tool
docker run --rm --entrypoint tar ghcr.io/aspen-chat/aspen-api:TAG -C /usr/share/aspen -c . \
  | tar -x -C ~/aspen-tool
~/aspen-tool/box/aspen-box --help
```

The images are published at `ghcr.io/aspen-chat/aspen-api` and `ghcr.io/aspen-chat/aspen-voice`, for
x86-64 and 64-bit ARM, and `--tag` names a release of both. To run images of your own, give
`--api-image` and `--voice-image` instead.

## Setting up

1. Describe the deployment. It asks for the domain, whether to keep people's files and the database
   on the machine or elsewhere, and the machine's internet upload:

   ```
   aspen-box init --tag TAG
   ```

   It writes everything into `~/aspen` (`--dir` for elsewhere), and prints the DNS records to make:
   the domain, `voice.` under it, and `media.` under it when files are kept on the machine.

2. Make those records at your DNS provider. If the machine is behind a router, forward the ports
   `init` listed.

3. Start it, and check it:

   ```
   aspen-box up
   aspen-box doctor
   ```

   `doctor` says what is wrong and what to do about it: the machine, the records, the certificates,
   and what the storage must refuse to serve. Run it whenever something does not work.

4. Create your account in the web client, then make it the first administrator
   ([Step 7](../../docs/operators/installing/7-first-administrator.md)):

   ```
   aspen-box run -- aspen-chat-server admin grant <username>
   ```

5. Measure what the voice server carries, set its `capacity` under `voice_servers` in
   `~/aspen/box.json`, and `aspen-box up`:

   ```
   aspen-box capacity
   ```

   It counts the internet upload you gave `init` (`rtc.upload_mbps`). Without one it counts the
   network card's speed, which a connection at home is far below: run a speed test on the machine,
   and give its upload.

## Day to day

| To                                                                             | Run                                                                                                                                                                                                                                     |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Move to a new release                                                          | `aspen-box upgrade --tag TAG`. The API does not stop: the new server starts beside the old one, which stops once the new one answers. Calls in progress end and rejoin. If the new release does not start, the old one goes on serving. |
| Back up the database                                                           | `aspen-box backup`, daily, from cron. It keeps the last seven in `~/aspen/backups`. **Copy them off the machine.**                                                                                                                      |
| Restore it                                                                     | `aspen-box restore FILE`. Everything since that backup is lost.                                                                                                                                                                         |
| Change a setting                                                               | Edit `~/aspen/box.json`, then `aspen-box up`, which restarts only what the change touches.                                                                                                                                              |
| Run an operator command                                                        | `aspen-box run -- aspen-chat-server …`                                                                                                                                                                                                  |
| See what is running, or why something stopped                                  | `aspen-box status`, `aspen-box logs [SERVICE]`                                                                                                                                                                                          |
| Move to a deployment that grows ([on AWS](../aws/README.md#moving-a-box-here)) | `aspen-box export DIR` stops the box and writes its database and people's files into `DIR`, which the next deployment is made from. `aspen-box up` starts the box again.                                                                |

People's files are not in the database's backups. Kept on the machine, they are in
`~/aspen/data/garage`: copy that directory too, with the deployment stopped (`aspen-box down`).

## DNS records

The deployment needs an `A` record (and an `AAAA` record, where the machine has IPv6) for each of
its names, which `init` prints. Write them at your DNS provider, and again whenever the machine's
address changes: a home connection's often does, and your provider's dynamic DNS can follow it.
`aspen-box doctor` says which records are missing or point elsewhere, and what each should say.

## Where people's files are kept

- **On the machine** (the default). A small object store, Garage, runs beside everything else and
  Caddy serves it at `media.<domain>`, passing only what clients may read. Nothing to sign up for.
  The files are as safe as the machine's disk.
- **In a bucket elsewhere** (`--storage s3`): Cloudflare R2, AWS S3, or anything that speaks S3. You
  give its address and keys, and **you set up its read path**, which must serve four prefixes and
  nothing else, to any origin
  ([The storage's read path](../../docs/operators/installing/storage-read-path.md)).
  `aspen-box doctor` checks that it refuses the rest, and that it lets the apps read files. On an
  AWS machine whose role may use the bucket, `--s3-role` gives no keys, and the server takes the
  machine's role.

## Where the database is kept

- **On the machine** (the default). PostgreSQL runs beside everything else, listening on loopback,
  with its files in `~/aspen/data/postgres`.
- **Elsewhere**: a PostgreSQL you run or rent. Answer `elsewhere` when `init` asks, and give its
  address as `postgres://user:password@host:5432/database`. For an existing box, set
  `"database": {"kind": "external", "url": "…", "ca_file": null}` in `box.json`.

For a database elsewhere:

- **It must be PostgreSQL 18 or later**, and empty the first time. `aspen-box up` checks the version
  before anything touches it. Its user must be allowed to create the `pg_trgm` and `btree_gin`
  extensions, as a database's owner is.
- **Its certificate is checked** unless the address says otherwise with `sslmode`. Providers whose
  certificates come from a public authority need nothing more. For one with an authority of its own
  (AWS RDS, or your own server), give its file as `ca_file`, or to `init` as `--database-ca`.
- **Every query crosses a network**, so keep the database near the machine.
- `aspen-box backup` and `restore` work on it too, over that connection. `restore` fills it only
  when it holds no tables: emptying a database you keep elsewhere is yours to do. Your provider's
  own backups are the better first line.
- **To move a box's own database out**: `aspen-box backup`, change `box.json` to the database
  elsewhere, then `aspen-box restore` that backup.

Given on a command line, the address shows its password to anyone who can list the machine's
processes for that moment. Answer `init`'s question instead, or put it in `ASPEN_BOX_DATABASE_URL`.

## Voice servers on other machines

A voice server elsewhere carries calls for people nearer it than the deployment's machine, or more
calls than one machine can. Each is a box of its own, holding the voice server alone.

1. On the deployment's box, name it, and give a guess at its capacity:

   ```
   aspen-box voice add voice-2 --capacity 50
   ```

   This registers it, gives it a NATS user of its own, and writes `voice-voice-2.join`, which holds
   everything the other machine needs, its NATS password among it. The first voice server elsewhere
   also opens the deployment's NATS to the network: it listens on TCP 4222, over TLS from an
   authority the box makes. Forward that port to the deployment's machine if it is behind a router,
   and let only the voice servers' machines reach it where you can.

2. On the other machine, with the tool and Docker:

   ```
   aspen-box init-voice --join voice-voice-2.join --tag TAG
   aspen-box up
   aspen-box doctor
   ```

   Then delete the join file. Make the record `init-voice` prints (`voice-2.<domain>`), and forward
   the ports it lists.

3. Measure its capacity there (`aspen-box capacity`), and set it under `voice_servers` in the
   deployment's `box.json`.

`aspen-box voice list` lists them, and `aspen-box voice remove voice-2` removes one: it is taken out
of the registry once it holds no calls, its NATS user goes, and NATS keeps to the machine again when
it was the last. Upgrade each voice server's box with `aspen-box upgrade --tag` on its own machine,
after the deployment's.

## At home

A computer at home works, with more to get right than a rented one:

- **An SSD, not an SD card.** A database wears an SD card out, and a card loses recent writes when
  power is cut. `doctor` says when the data is on one.
- **The router** must forward TCP 80 and 443, UDP 3478, and UDP and TCP 40000 to 40199 to the
  machine.
- **Some providers share one address between customers**, and then nothing can be forwarded.
  `doctor` shows the address the internet sees; if your router's own outside address is a different
  one, this is why. IPv6 alone does not help people whose own network lacks it.
- **An address that changes**: run `aspen-box refresh-address` from cron every few minutes, which
  has the voice server announce the new one, and keep the DNS records following it with your
  provider's dynamic DNS.
- **Calls are limited by your upload speed.** Each participant takes about 3 Mbit/s of upload when
  everyone sends video, so 20 Mbit/s carries about four or five. Give it to `init`, or as
  `rtc.upload_mbps` in `box.json`.
- **Mail** from a home address is refused nearly everywhere. Give `box.json` an `email` with a mail
  provider's SMTP address ([Email](../../docs/operators/installing/3-configuring.md#email)).

## What it does not do yet

- **A machine with no address the internet reaches.** There is no relay for one.

## Trying it without a machine's own network

Where Docker cannot give containers the machine's network (rootless Docker), or its loopback ports
are taken, `ASPEN_BOX_COMPOSE_FILES` names further Compose files laid over `compose.yaml`, and
`ASPEN_BOX_PROJECT` names the Compose project, so that boxes stand side by side. A file that gives
every service `network_mode: "service:<one container>"` stands in for a machine, reachable only from
inside that container's network; `scripts/check_box.py` runs every box it checks that way.
