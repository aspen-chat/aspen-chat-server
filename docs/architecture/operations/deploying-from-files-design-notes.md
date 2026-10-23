# Deploying from files: design notes

Why `deploy/` is the way it is. How to use it is in `deploy/README.md` and `deploy/aws/README.md`.

## The images

- **One build stage holds both toolchains.** The web client's attributions page lists the servers'
  crates, which its build asks cargo for, and reads their licences from cargo's downloads. A
  Node-only stage cannot build it.
- **`target/` is not kept between builds.** Cargo decides what to rebuild by modification times. A
  kept `target/` is shared by every checkout built on the machine, and a checkout whose files are
  older than the last build is taken for already built: the image would hold another checkout's
  servers, with no error. Cargo's downloads are kept, which are named by their contents.
- **The images hold no configuration, and the `api` image names its own web client** through
  `ASPEN_WEB_CLIENT__DIR`. The environment overrides `aspen.toml`, so whatever file a deployment
  mounts, the server serves the web client it was built with.
- **The image is told its commit** (`ASPEN_COMMIT`, `ASPEN_MODIFIED`), since `.git` is left out of
  the build: what is sent to the builder is then the files alone, and an image built from a checkout
  with changes says so in About Aspen.

## Configuration reaches a container as a whole file

Each container is handed its `aspen.toml`, `voice_server.toml`, or NATS configuration as one secret,
and writes it into its working directory before starting the server.

- The files are the ones the operator guide documents, so the guide's every page applies to a
  deployment made this way.
- Environment variables can say everything, lists and tables as JSON, but a file is what the
  operator guide shows and what a person reads back when something is wrong.
- Each task definition names one version of its secret. A changed file is then a changed task
  definition, which is what makes a service roll out, and a task that starts later reads the file it
  was deployed with rather than whatever is newest.
- An operator command run in place of a task's command starts through the same entry point, which is
  how `admin grant` finds the deployment's `aspen.toml` with nothing copied by hand.

## OpenTofu on AWS

- **OpenTofu** rather than Terraform: it is under the licence Aspen is (MPL-2.0), reads the same
  language and providers, and encrypts its state itself, which matters because the state holds every
  password.
- **Fargate for the API servers and NATS.** They keep nothing on disk and take TCP alone, so there
  is no machine worth owning beneath them.
- **A machine each for the voice servers.** People's devices send media to a voice server's own
  address, over a thousand UDP ports. No load balancer carries that, so each has an Elastic IP and
  the host's network. ECS still runs its container, so it is released the way the API servers are.
- **Signalling goes through the load balancer**, which already holds a certificate for every name
  under the domain. A voice server then needs no certificate of its own to renew.
- **The servers are in public subnets.** The API servers must reach the internet directly (link
  previews, other deployments, push services). From private subnets that takes a NAT gateway in each
  zone. Their security groups take connections from the load balancer alone.
- **The read path is CloudFront with origin access control.** The bucket's policy lets the
  distribution read the four public prefixes, and gives no one permission to list. Everything else
  answers 403, without a proxy rule to keep right.
- **The bucket allows uploads from any origin.** A presigned URL is what allows an upload, so the
  origin adds nothing, and the apps' origins (`null`, `capacitor://localhost`, `https://localhost`)
  need no list kept in step with them.
- **Voice server ids are made by the configuration** (`voice-servers add --id`). The id goes into
  the voice server's file and its NATS user's permissions, both written before anything runs that
  could register it.
- **Migrations run as a task during the apply, not in each API server's container.** A release whose
  migrations fail then stops before any server is replaced, and the old ones go on serving. The same
  task registers the voice servers.
- **The servers reach the bucket as their task's role**, given no key pair. A key pair would be a
  password in `aspen.toml` that never expires, and the role's credentials are short-lived and
  renewed by the SDK.
- **The images are pulled from where they are published**, without signing in. Repositories of the
  deployment's own would need every release built and pushed before an apply could use it.
- **IPv6 wherever the internet meets the deployment**: the load balancer, the voice servers, the
  read path, and the servers' own requests out. People on networks with IPv6 alone reach it without
  a translator. The database and Valkey keep to IPv4 in subnets of their own, which nothing outside
  the network reaches.
- **Fargate tasks take IPv6 through an account setting** (`dualStackIPv6`), the only way ECS offers.
  It is the default for the account and region, so the README says so before the first apply.
- **Each voice server's IPv6 address follows from its name.** It is announced to clients in its
  file, which must not change when the machine is replaced, and AWS gives a new machine a new
  address unless told one. Derived from the name's hash, adding a server moves no other; derived
  from the servers' order, it would replace machines, and end calls, on every addition.
- **Voice servers ignore newer machine images** until replaced by name. Otherwise whichever apply
  came after AWS published an image would replace every voice server and end its calls.

## One machine on AWS

`deploy/aws-single` trades staying up for cost: it is a box, on one EC2 machine, with its files in
S3.

- **What costs money in the larger configuration is mostly what keeps it up**: a second API server,
  the load balancer, and managed PostgreSQL and Valkey. A deployment of a few dozen people that may
  be down for some minutes needs none of them.
- **The box, not containers of its own.** Everything a deployment on one machine needs (Caddy, the
  two places for the API server, upgrades that do not stop it, backups, `doctor`) is the box's
  already, and tried on any machine. The configuration makes the machine, its addresses, its volume,
  and the bucket, and hands the box the answers `init` would ask for.
- **A release is still a `tofu apply`.** It runs `aspen-release` on the machine through Session
  Manager, which takes the tool from the release's own image and makes the box or upgrades it. No
  port but the deployment's is open, and no key to the machine exists.
- **box.json is the configuration after the first release.** The variables give only what the box
  cannot change later (the domain, the bucket, the addresses) and the release. Everything else is
  changed on the box, as on any other, so there is one way to change a setting rather than two that
  disagree.
- **The box is on a volume of its own**, so replacing the machine keeps the database, the passwords,
  and the certificates (Let's Encrypt issues only so many for a name each week). Docker waits for
  the volume's mount: a container started without it would make an empty database on the machine's
  own disk.
- **Backups are snapshots of that volume.** One taken while PostgreSQL runs restores the way a
  database that lost power does, which PostgreSQL is built to survive.
- **Storage stays in S3 behind CloudFront.** It costs by use, which at this size is cents, keeping
  attachments on the machine would put them on the one disk that can be lost, and a deployment that
  moves to the larger configuration takes the bucket over where it is.
- **Ubuntu, for Docker with Compose from its own packages.** The box needs Compose 2.20 or later,
  which Amazon Linux does not package.

## Moving a box to the larger configuration

- **A box is exported whole, by the box** (`aspen-box export`): the database, people's files, and
  what the next deployment must know (its voice servers' names). The box knows where its database
  and its files are, wherever they are kept; the migration script needs to know only the directory.
- **From `deploy/aws-single`, only the database is copied.** Its files are in a bucket the larger
  deployment takes over where it is. Everything else a deployment keeps is in one of the two, or is
  not worth keeping (Valkey, NATS).
- **A dump and a restore, with the old deployment stopped.** Someone on one machine has accepted
  being down for a release, and a small database copies in less time than that. Replicating instead
  would need a network path between the two deployments and a database that takes no schema change
  meanwhile, for minutes saved once.
- **The restore runs before the new deployment's first release**, while it has a database and no
  servers: the state an apply without `image_tag` leaves. The release then finds the old
  deployment's tables and brings them up to date, exactly as an upgrade does.
- **The restore is one transaction that refuses a database with tables in it.** Run twice, or after
  a release made empty tables, it changes nothing rather than mixing two databases.
- **The dump travels through the bucket**, under a prefix the read path does not serve and a rule
  deletes after a week. The new deployment's database is closed to everything outside its network,
  and the bucket is the one thing both sides already reach.
- **The restore task is stock images in two containers**, one for S3 and one for PostgreSQL's tools,
  sharing a volume. Neither server's image has `pg_restore`, and an image of our own for one use
  would have to be built and published at every release to be there when wanted. Its role allows the
  one object.
- **The larger deployment imports the bucket and the distribution, and makes the rest of the storage
  anew** (its certificate, access control). Those two cannot be made twice: the bucket holds the
  files, and `media.<domain>` belongs to one distribution at a time. The bucket's policy names the
  distribution, so both deployments read through it while they overlap.
- **The old configuration forgets what was taken over instead of destroying it** (`tofu state rm`,
  from the list the media module gives as `handed_over`). A `removed` block cannot do it: the module
  still declares those resources for deployments that are staying.
- **The name changes hands in the release's apply** (`public_dns`), the read path's with it when the
  files were copied rather than taken over. Until then the larger deployment exists beside the old
  one without the name, so everything slow (the database, the certificates, the load balancer) is
  made while the old one still serves.
- **The old data volume is forgotten too, not deleted.** It is the old database, and the way back if
  the move turns out badly a day later.
- **The script does what is not an apply, and prints the apply to run.** Plans that import or take
  over a name are the ones a person should read.

## The box

`deploy/box` is the one-machine deployment with nothing of any provider in it, for the people Aspen
most wants to reach: a small rented server, or a small computer at home.

- **One tool that asks a few questions, rather than files to edit.** It makes every password, writes
  every file, and starts things in the order they need. What it was told is one JSON file, which is
  also how a setting is changed later.
- **Python's standard library and Docker Compose, and nothing to install.** A small computer's
  system has both, or one package away. A compiled tool would need building for each architecture
  before it could help anyone start.
- **Every container on the machine's network, every service on loopback.** The voice server's media
  is a range of UDP ports, which Docker publishes one process a port. With everything beside it, no
  service is reachable from another machine whatever the firewall says, which matters most where
  there is no firewall but a home router.
- **Two places for the API server, one running.** An upgrade, or a changed `aspen.toml`, starts the
  server in the other place and stops the first once the second answers; Caddy gives a request that
  finds one gone to the other. A small deployment then has no outage for a release, and a release
  that does not start changes nothing. The cost is a second server's memory for the seconds both
  run.
- **What routes requests asks `GET /health`; what starts a server asks `GET /health/ready`.** Caddy
  and the load balancer need to know a server is there. Were they to ask whether it reaches its
  database, a database outage would take every server out at once, and people would get a proxy's
  error in place of the server's own, which says what is wrong. An upgrade waits for readiness
  instead, before it stops the server it replaces.
- **Garage for files kept on the machine**, since it is one small process that keeps to a few
  megabytes idle, and it makes its own bucket and key pair from its environment. Its S3 API and its
  read path share `media.<domain>`: paths under the bucket's name are the S3 API, which answers only
  what the server signed, and the four public prefixes are the read path. One name to make a record
  for, not two.
- **Uploads' CORS is answered by Caddy**, with any origin allowed, as on S3. Setting it in the store
  would take an S3 client the box otherwise has no use for.
- **`doctor` is where the know-how goes.** Most of what goes wrong with a first deployment is
  outside the servers: a record that points elsewhere, a port not forwarded, a certificate not yet
  issued, storage that serves too much. It checks each, and gives the record to write or the command
  to run. What it cannot see from inside the network (whether media ports are reachable) it says it
  has not checked, and how to.
- **The templates hold placeholders alone.** OpenTofu and Python each fill `${name}` and nothing
  more, so one set of files serves the box and both AWS configurations. A template with conditions
  or loops could be rendered by only one of them.
- **The box makes no DNS records.** A provider's API is often closed to the people the box is for
  (an account of few domains, an address that changes and so cannot be allow-listed), which is when
  it would be most wanted. `init` prints the records and `doctor` checks them.
- **A voice server elsewhere is a box of its own, told everything in one file.** The deployment's
  box registers it and makes its NATS user, so the other machine needs no access to the deployment's
  database or secrets: only its own NATS password, its id, and the authority to trust NATS by.
- **NATS opens to the network only while a voice server elsewhere needs it**, and then only over TLS
  from an authority the box makes for itself. A certificate from a public authority would need its
  name renewed for a port no browser asks for; the voice servers are told the authority in their
  join file instead. The box's own servers reach NATS over loopback, also over TLS then, since NATS
  asks it of every client or none.
- **The tool is a package of modules, launched by one small script**, and is copied into every `api`
  image. A machine with Docker and no checkout takes it out of the release it will run, so the tool
  and the images always match.
- **A database elsewhere is the same box with one service fewer.** PostgreSQL is in a Compose
  profile, like the bundled storage, and the servers' dependence on it is one that may be absent.
  Nothing else about the box changes, so someone can start with everything on the machine and move
  the database out later with a backup and a restore.
- **Its certificate is checked unless the address asks otherwise.** On the machine the database is
  on loopback; elsewhere every query crosses a network, where an unchecked connection can be read
  and asked for its password. The default is the safe one, and `doctor` says so when the address has
  chosen another.
- **PostgreSQL's tools run from its image, told where the database is by their environment.**
  Neither server's image has `pg_dump` or `psql`, and the PostgreSQL image is the one piece sure to
  match the version asked for. The password is in the container's environment, never in an argument,
  which every user of the machine can read.
- **The version is checked before a release meets the database.** A migration failing on a function
  PostgreSQL 17 lacks says nothing a person can act on.
- **`restore` never empties a database kept elsewhere.** It drops and remakes the box's own, which
  is the box's to drop. Elsewhere it restores in one transaction, which a database with tables in it
  refuses whole.
