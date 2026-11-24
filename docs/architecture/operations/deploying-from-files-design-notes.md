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
- **Each kind of task has an execution role of its own**, allowed to read only its own secret: the
  servers `aspen.toml`, NATS its configuration, the restore the database's password, and each voice
  server its own file. A voice server faces the internet on a machine of its own, where whoever
  takes it over can reach the ECS agent's credentials; with one role for every task, that would hand
  them `aspen.toml`, the database's and NATS's passwords among it.

## OpenTofu on AWS

- **OpenTofu** rather than Terraform: it is under the licence Aspen is (MPL-2.0), reads the same
  language and providers, and encrypts its state itself, which matters because the state holds every
  password.
- **Fargate for the API servers and NATS.** They keep nothing on disk and take TCP alone, so there
  is no machine worth owning beneath them.
- **A machine each for the voice servers.** People's devices send media to a voice server's own
  address, over UDP and TCP ports of its own. No load balancer carries that, so each has an Elastic
  IP and the host's network. ECS still runs its container, so it is released the way the API servers
  are. On the host's network the instance metadata service would answer the voice server with the
  machine role's credentials, so the voice image runs as a user of its own (10002), which every
  machine running it refuses that address from boot (`deploy/aws/machine/voice-imds.service`).
- **The load balancer asks `GET /health`, not `GET /health/ready`.** It needs to know a server is
  there. Were it to ask whether the server reaches its database, a database outage would take every
  server out at once, and people would get the load balancer's error in place of the server's own,
  which says what is wrong.
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

## Moving Ultra Lite to AWS

- **Ultra Lite is exported whole, by itself** (`aspen-ul export`): the database, people's files, and
  what the next deployment must know (its address and its voice server's name). It knows where its
  database and files are; the migration script needs to know only the directory.
- **A dump and a restore, with the old deployment stopped.** Someone on one machine has accepted
  being down for a release, and a small database copies in less time than that. Replicating instead
  would need a network path between the two deployments and a database that takes no schema change
  meanwhile, for minutes saved once.
- **The files are uploaded by the server's own command** (`aspen-chat-server storage import`, run
  from the API image), which sets each object's type and disposition as the server would have. A
  copy of the files alone would lose them, since an object's key has no extension to tell its type
  by.
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
- **The name changes hands in the release's apply** (`public_dns`), the read path's with it. Until
  then the larger deployment exists beside the old one without the name, so everything slow (the
  database, the certificates, the load balancer) is made while the old one still serves.
- **The script does what is not an apply, and prints the apply to run.** Plans that take over a name
  are the ones a person should read.

## The templates

- **The templates hold placeholders alone.** OpenTofu fills `${name}` and nothing more, so what a
  file holds is read from its template, and a program of any language can fill one the same way.
