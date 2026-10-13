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
- **Migrations run as a task during the apply, not in each API server's container.**
  `aspen-migrate` takes no lock, so servers starting together would run it together.
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
