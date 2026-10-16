# Deploying from files: design notes

Why `deploy/` is the way it is. How to use it is in `deploy/README.md` and
`deploy/aws/README.md`.

## The images

- **One build stage holds both toolchains.** The web client's attributions page lists the
  servers' crates, which its build asks cargo for, and reads their licences from cargo's
  downloads. A Node-only stage cannot build it.
- **`target/` is not kept between builds.** Cargo decides what to rebuild by modification
  times. A kept `target/` is shared by every checkout built on the machine, and a checkout whose
  files are older than the last build is taken for already built: the image would hold another
  checkout's servers, with no error. Cargo's downloads are kept, which are named by their
  contents.
- **The images hold no configuration, and the `api` image names its own web client** through
  `ASPEN_WEB_CLIENT__DIR`. The environment overrides `aspen.toml`, so whatever file a deployment
  mounts, the server serves the web client it was built with.
- **The built image knows no commit.** `.git` is left out of the build, so About Aspen shows the
  version alone.

## Configuration reaches a container as a whole file

Each container is handed its `aspen.toml`, `voice_server.toml`, or NATS configuration as one
secret, and writes it into its working directory before starting the server.

- The files are the ones the operator guide documents, so the guide's every page applies to a
  deployment made this way.
- Environment variables alone cannot say everything: a setting that is a list
  (`[rate_limits] trusted_proxies`) has no form there in `aspen.toml`'s loader, and the server
  refuses to start without the file.
- Each task definition names one version of its secret. A changed file is then a changed task
  definition, which is what makes a service roll out, and a task that starts later reads the
  file it was deployed with rather than whatever is newest.
- An operator command run in place of a task's command starts through the same entry point,
  which is how `admin grant` finds the deployment's `aspen.toml` with nothing copied by hand.

## OpenTofu on AWS

- **OpenTofu** rather than Terraform: it is under the licence Aspen is (MPL-2.0), reads the same
  language and providers, and encrypts its state itself, which matters because the state holds
  every password.
- **Fargate for the API servers and NATS.** They keep nothing on disk and take TCP alone, so
  there is no machine worth owning beneath them.
- **A machine each for the voice servers.** People's devices send media to a voice server's own
  address, over a thousand UDP ports. No load balancer carries that, so each has an Elastic IP
  and the host's network. ECS still runs its container, so it is released the way the API
  servers are.
- **Signalling goes through the load balancer**, which already holds a certificate for every
  name under the domain. A voice server then needs no certificate of its own to renew.
- **The servers are in public subnets.** The API servers must reach the internet directly (link
  previews, other deployments, push services). From private subnets that takes a NAT gateway in
  each zone. Their security groups take connections from the load balancer alone.
- **The read path is CloudFront with origin access control.** The bucket's policy lets the
  distribution read the four public prefixes, and gives no one permission to list. Everything
  else answers 403, without a proxy rule to keep right.
- **The bucket allows uploads from any origin.** A presigned URL is what allows an upload, so
  the origin adds nothing, and the apps' origins (`null`, `capacitor://localhost`,
  `https://localhost`) need no list kept in step with them.
- **Voice server ids are made by the configuration** (`voice-servers add --id`). The id goes
  into the voice server's file and its NATS user's permissions, both written before anything
  runs that could register it.
- **Migrations run as a task during the apply, not in each API server's container.** A release
  whose migrations fail then stops before any server is replaced, and the old ones go on
  serving. The same task registers the voice servers.
- **The storage key pair is an IAM user's.** `[media.s3]` takes a key pair only, so the servers
  cannot use their task's role.
- **Voice servers ignore newer machine images** until replaced by name. Otherwise whichever
  apply came after AWS published an image would replace every voice server and end its calls.

## One machine on AWS

`deploy/aws-single` trades staying up for cost: everything the larger configuration spreads
over managed services runs as containers on one machine.

- **What costs money in the larger configuration is mostly what keeps it up**: a second API
  server, the load balancer, and managed PostgreSQL and Valkey. A deployment of a few dozen
  people that may be down for some minutes needs none of them.
- **ECS still runs the containers**, on the one machine. A release is then the same `tofu
  apply` as in the larger configuration, with the same release task and `run_task.py`, and
  nothing is copied to the machine or run on it by hand.
- **Every container is on the machine's own network, and every service listens on loopback.**
  The voice server needs the host's network for its media ports, and with the rest beside it
  the servers reach PostgreSQL, NATS, and Valkey at 127.0.0.1: no password or event crosses a
  network, and no container port is published past a firewall.
- **Caddy terminates TLS** in the load balancer's place, and renews its own certificates. It
  keeps them on the data volume, since Let's Encrypt issues only so many for a name each week
  and a replaced machine would otherwise ask again.
- **The database's files are on a volume of their own**, so replacing the machine keeps them.
  The machine's first boot waits for that volume before the ECS agent may start anything:
  PostgreSQL started without it would make an empty database on the machine's own disk.
- **Backups are snapshots of that volume.** One taken while PostgreSQL runs restores the way a
  database that lost power does, which PostgreSQL is built to survive.
- **Storage stays in S3 behind CloudFront.** It costs by use, which at this size is cents, and
  keeping attachments on the machine would put them on the one disk that can be lost.
- **NATS is told how much memory its streams may reserve** (`nats_stream_mib`), and the servers
  size the streams to it through its monitoring port, rather than the gigabyte they ask for
  when they cannot see a limit.
- **The configuration files are in Secrets Manager rather than Parameter Store**, which would
  be free: the machine's own role may read every parameter, so anything on the machine could
  read them without going through ECS.
- **The two configurations share modules for the cluster and the storage**, and the templates.
  The rest differs in kind, not in size, so it is written twice rather than switched on a
  variable.

## Moving from one machine to the larger configuration

- **Only the database is copied.** Everything else a deployment keeps is in the bucket, which
  the larger deployment takes over where it is, or is not worth keeping (Valkey, NATS).
- **A dump and a restore, with the old deployment stopped.** Someone on one machine has
  accepted being down for a release, and a small database copies in less time than that.
  Replicating instead would need a network path between the two deployments and a database
  that takes no schema change meanwhile, for minutes saved once.
- **The restore runs before the new deployment's first release**, while it has a database and
  no servers: the state an apply without `image_tag` leaves. The release then finds the old
  deployment's tables and brings them up to date, exactly as an upgrade does.
- **The restore is one transaction that refuses a database with tables in it.** Run twice, or
  after a release made empty tables, it changes nothing rather than mixing two databases.
- **The dump travels through the bucket**, under a prefix the read path does not serve and a
  rule deletes after a week. The two deployments are in separate networks with their databases
  closed to everything outside, and the bucket is the one thing both already reach.
- **The export and restore tasks are stock images in two containers**, one for PostgreSQL's
  tools and one for S3, sharing a volume. Neither server's image has `pg_dump`, and an image
  of our own for one use would have to be built and pushed at every release to be there when
  wanted.
- **Each task's role allows the one object.** The servers' own key pair could write it, but it
  is inside `aspen.toml` and allows the whole bucket.
- **The larger deployment imports the bucket and the distribution, and makes the rest of the
  storage anew** (its key pair, certificate, access control). Those two cannot be made twice:
  the bucket holds the files, and `media.<domain>` belongs to one distribution at a time. The
  bucket's policy names the distribution, so both deployments read through it while they
  overlap.
- **The old configuration forgets what was taken over instead of destroying it** (`tofu state
  rm`, from the list the media module gives as `handed_over`). A `removed` block cannot do it:
  the module still declares those resources for deployments that are staying.
- **The name changes hands in the release's apply** (`public_dns`). Until then the larger
  deployment exists beside the old one without the name, so everything slow (the database, the
  certificates, the load balancer) is made while the old one still serves.
- **The old data volume is forgotten too, not deleted.** It is the old database, and the way
  back if the move turns out badly a day later.
- **The script does what is not an apply, and prints the apply to run.** Plans that import or
  take over a name are the ones a person should read.

## The box

`deploy/box` is the one-machine deployment with nothing of any provider in it, for the people
Aspen most wants to reach: a small rented server, or a small computer at home.

- **One tool that asks a few questions, rather than files to edit.** It makes every password,
  writes every file, and starts things in the order they need. What it was told is one JSON
  file, which is also how a setting is changed later.
- **Python's standard library and Docker Compose, and nothing to install.** A small computer's
  system has both, or one package away. A compiled tool would need building for each
  architecture before it could help anyone start.
- **Every container on the machine's network, every service on loopback.** The voice server's
  media is a range of UDP ports, which Docker publishes one process a port. With everything
  beside it, no service is reachable from another machine whatever the firewall says, which
  matters most where there is no firewall but a home router.
- **Two places for the API server, one running.** An upgrade, or a changed `aspen.toml`, starts
  the server in the other place and stops the first once the second answers; Caddy gives a
  request that finds one gone to the other. A small deployment then has no outage for a
  release, and a release that does not start changes nothing. The cost is a second server's
  memory for the seconds both run.
- **What routes requests asks `GET /health`; what starts a server asks `GET /health/ready`.**
  Caddy and the load balancer need to know a server is there. Were they to ask whether it
  reaches its database, a database outage would take every server out at once, and people would
  get a proxy's error in place of the server's own, which says what is wrong. An upgrade waits
  for readiness instead, before it stops the server it replaces.
- **Garage for files kept on the machine**, since it is one small process that keeps to a
  few megabytes idle, and it makes its own bucket and key pair from its environment. Its S3
  API and its read path share `media.<domain>`: paths under the bucket's name are the S3 API,
  which answers only what the server signed, and the four public prefixes are the read path.
  One name to make a record for, not two.
- **Uploads' CORS is answered by Caddy**, with any origin allowed, as on S3. Setting it in the
  store would take an S3 client the box otherwise has no use for.
- **`doctor` is where the know-how goes.** Most of what goes wrong with a first deployment is
  outside the servers: a record that points elsewhere, a port not forwarded, a certificate not
  yet issued, storage that serves too much. It checks each, and gives the record to write or
  the command to run. What it cannot see from inside the network (whether media ports are
  reachable) it says it has not checked, and how to.
- **The templates hold placeholders alone.** OpenTofu and Python each fill `${name}` and
  nothing more, so one set of files serves the box and both AWS configurations. A template
  with conditions or loops could be rendered by only one of them.
- **The AWS one-machine configuration is not built on the box yet.** It runs its containers
  with ECS, which the box cannot use elsewhere. Rebuilt as a machine that runs the box, it
  would share everything but making the machine.
- **Making DNS records is offered, never needed.** A token that can edit someone's DNS is a lot
  to ask, so the box works without one and says which records to write. Between that and
  keeping a token there is a middle: a token in the environment of one command, written
  nowhere, for someone who will let the tool act once but not hold the key.
- **A kept token is what follows a changing address**, which is why keeping one is offered at
  all: a home connection's address changes, and `refresh-address` can then correct the records
  as well as what the voice server announces.
- **The tool writes only the records it lists and deletes none.** A name with two records of
  one type is someone's choice or someone's mistake, and either way not the tool's to settle.
- **The token is checked when it is given**, against the zone the domain is in, so a token for
  the wrong zone is found at `init` rather than when certificates fail to arrive.
- **A database elsewhere is the same box with one service fewer.** PostgreSQL is in a Compose
  profile, like the bundled storage, and the servers' dependence on it is one that may be
  absent. Nothing else about the box changes, so someone can start with everything on the
  machine and move the database out later with a backup and a restore.
- **Its certificate is checked unless the address asks otherwise.** On the machine the database
  is on loopback; elsewhere every query crosses a network, where an unchecked connection can be
  read and asked for its password. The default is the safe one, and `doctor` says so when the
  address has chosen another.
- **PostgreSQL's tools run from its image, told where the database is by their environment.**
  Neither server's image has `pg_dump` or `psql`, and the PostgreSQL image is the one piece
  sure to match the version asked for. The password is in the container's environment, never
  in an argument, which every user of the machine can read.
- **The version is checked before a release meets the database.** A migration failing on a
  function PostgreSQL 17 lacks says nothing a person can act on.
- **`restore` never empties a database kept elsewhere.** It drops and remakes the box's own,
  which is the box's to drop. Elsewhere it restores in one transaction, which a database with
  tables in it refuses whole.
