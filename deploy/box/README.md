# Aspen on one machine

`aspen-box` runs a whole deployment on one Linux machine with Docker: a small server you rent,
or a small computer you own. It is for a community of tens of people.

**New and lightly tried.** It has been started, upgraded, backed up, and restored on one x86
machine, with the machine's network stood in for. It has not yet run on an ARM computer, on a
real domain, or behind a home router.

## What it needs

| | |
| --- | --- |
| A machine | 64-bit Linux, 2 GiB of memory or more (4 is comfortable), and an SSD. Not an SD card: see [At home](#at-home). |
| Docker | The system's, with Compose 2.20 or later. Not rootless Docker, whose containers nothing can reach. |
| Python 3.9 | Nothing else: the tool uses only what comes with it. |
| A domain | One name for the deployment, **for good** ([Choosing `public_url`](../../docs/operators/installing/3-configuring.md#choosing-public_url)), and the right to add records under it. |
| An address the internet reaches | The machine's own, or a router's that forwards to it. |
| The images | `api` and `voice`, built from [`../docker/Dockerfile`](../README.md#the-images) for the machine's architecture. Building them on a small computer takes hours; build them elsewhere and `docker save` them across, until there are published ones. |

## Setting up

1. Describe the deployment. It asks for the domain, and whether to keep people's files and the
   database on the machine or elsewhere:

   ```
   deploy/box/aspen-box init --api-image aspen-api:TAG --voice-image aspen-voice:TAG
   ```

   It writes everything into `~/aspen` (`--dir` for elsewhere), and prints the DNS records to
   make: the domain, `voice.` under it, and `media.` under it when files are kept on the
   machine.

2. Make those records, yourself or [with an API token](#dns-records). If the machine is behind
   a router, forward the ports `init` listed.

3. Start it, and check it:

   ```
   aspen-box up
   aspen-box doctor
   ```

   `doctor` says what is wrong and what to do about it: the machine, the records, the
   certificates, and what the storage must refuse to serve. Run it whenever something does
   not work.

4. Create your account in the web client, then make it the first administrator
   ([Step 7](../../docs/operators/installing/7-first-administrator.md)):

   ```
   aspen-box run -- aspen-chat-server admin grant <username>
   ```

5. Measure what the voice server carries, and set `voice.capacity` in `~/aspen/box.json` to
   it, then `aspen-box up`:

   ```
   aspen-box capacity
   ```

## Day to day

| To | Run |
| --- | --- |
| Move to a new release | `aspen-box upgrade --tag TAG`. The API does not stop: the new server starts beside the old one, which stops once the new one answers. Calls in progress end and rejoin. If the new release does not start, the old one goes on serving. |
| Back up the database | `aspen-box backup`, daily, from cron. It keeps the last seven in `~/aspen/backups`. **Copy them off the machine.** |
| Restore it | `aspen-box restore FILE`. Everything since that backup is lost. |
| Change a setting | Edit `~/aspen/box.json`, then `aspen-box up`, which restarts only what the change touches. |
| Run an operator command | `aspen-box run -- aspen-chat-server …` |
| See what is running, or why something stopped | `aspen-box status`, `aspen-box logs [SERVICE]` |

People's files are not in the database's backups. Kept on the machine, they are in
`~/aspen/data/garage`: copy that directory too, with the deployment stopped (`aspen-box down`).

## DNS records

The deployment needs an `A` record (and an `AAAA` record, where the machine has IPv6) for each
of its names. `aspen-box dns` lists them, with what each resolves to now. There are three ways
to make them, by how far you trust this tool with your DNS:

| | What you do | What the tool holds |
| --- | --- | --- |
| By hand | Write the records at your DNS provider, and again whenever the machine's address changes. | Nothing. This is what happens unless you choose otherwise. |
| A token for one command | `ASPEN_BOX_DNS_TOKEN=… aspen-box dns --apply --provider cloudflare` | The token for that command. It is written nowhere. |
| A token it keeps | Answer `cloudflare` when `init` asks, or put `"dns": {"provider": "cloudflare", "token": "…"}` in `box.json`. | The token, in `box.json`, which only your user can read. `aspen-box up` and `refresh-address` then keep the records right, which is what a home connection whose address changes needs. |

With a token, the tool makes or corrects the records `aspen-box dns` lists and touches no
other. It never deletes a record: where a name has two of one type it stops and says so.

Cloudflare is the one provider so far. Make the token at Cloudflare under API Tokens with the
"Edit zone DNS" template, limited to the one zone: it can then change that zone's DNS and
nothing else of your account. The records are made unproxied ("DNS only"), since the box gets
its own certificates and calls go straight to its address.

## Where people's files are kept

- **On the machine** (the default). A small object store, Garage, runs beside everything else
  and Caddy serves it at `media.<domain>`, passing only what clients may read. Nothing to sign
  up for. The files are as safe as the machine's disk.
- **In a bucket elsewhere** (`--storage s3`): Cloudflare R2, AWS S3, or anything that speaks
  S3. You give its address and keys, and **you set up its read path**, which must serve four
  prefixes and nothing else
  ([The storage's read path](../../docs/operators/installing/storage-read-path.md)).
  `aspen-box doctor` checks that it refuses the rest.

## Where the database is kept

- **On the machine** (the default). PostgreSQL runs beside everything else, listening on
  loopback, with its files in `~/aspen/data/postgres`.
- **Elsewhere**: a PostgreSQL you run or rent. Answer `elsewhere` when `init` asks, and give its
  address as `postgres://user:password@host:5432/database`. For an existing box, set
  `"database": {"kind": "external", "url": "…", "ca_file": null}` in `box.json`.

For a database elsewhere:

- **It must be PostgreSQL 18 or later**, and empty the first time. `aspen-box up` checks the
  version before anything touches it. Its user must be allowed to create the `pg_trgm` and
  `btree_gin` extensions, as a database's owner is.
- **Its certificate is checked** unless the address says otherwise with `sslmode`. Providers
  whose certificates come from a public authority need nothing more. For one with an authority
  of its own (AWS RDS, or your own server), give its file as `ca_file`, or to `init` as
  `--database-ca`.
- **Every query crosses a network**, so keep the database near the machine.
- `aspen-box backup` and `restore` work on it too, over that connection. `restore` fills it
  only when it holds no tables: emptying a database you keep elsewhere is yours to do. Your
  provider's own backups are the better first line.
- **To move a box's own database out**: `aspen-box backup`, change `box.json` to the database
  elsewhere, then `aspen-box restore` that backup.

Given on a command line, the address shows its password to anyone who can list the machine's
processes for that moment. Answer `init`'s question instead, or put it in
`ASPEN_BOX_DATABASE_URL`.

## At home

A computer at home works, with more to get right than a rented one:

- **An SSD, not an SD card.** A database wears an SD card out, and a card loses recent writes
  when power is cut. `doctor` says when the data is on one.
- **The router** must forward TCP 80 and 443, UDP 3478, and UDP and TCP 40000 to 40199 to the
  machine.
- **Some providers share one address between customers**, and then nothing can be forwarded.
  `doctor` shows the address the internet sees; if your router's own outside address is a
  different one, this is why. IPv6 alone does not help people whose own network lacks it.
- **An address that changes**: run `aspen-box refresh-address` from cron every few minutes.
  With [a DNS token it keeps](#dns-records) it corrects the records too; otherwise keep them
  following the address with your provider's dynamic DNS.
- **Calls are limited by your upload speed**, which `aspen-box capacity` cannot see: it reads
  the network card's. Each participant takes about 3 Mbit/s of upload when everyone sends
  video, so 20 Mbit/s carries about four or five.
- **Mail** from a home address is refused nearly everywhere. Give `box.json` an `email` with a
  mail provider's SMTP address ([Email](../../docs/operators/installing/3-configuring.md#email)).

## What it does not do yet

- **Published images.** You build them, for now.
- **DNS providers other than Cloudflare.** Elsewhere you write the records by hand.
- **A machine with no address the internet reaches.** There is no relay for one yet.
- **Moving to the larger configuration with files kept on the machine.** A backup restores
  into it ([Moving from one machine](../aws/README.md#moving-from-one-machine) does this for
  the AWS one-machine deployment), but nothing yet copies the bundled storage across.
- **More than one voice server**, or one somewhere else.

## Trying it without a machine's own network

Where Docker cannot give containers the machine's network (rootless Docker), or its loopback
ports are taken, `ASPEN_BOX_COMPOSE_FILES` names further Compose files laid over
`compose.yaml`. One that gives every service `network_mode: "service:<one container>"` stands
in for the machine, reachable only from inside that container's network.
