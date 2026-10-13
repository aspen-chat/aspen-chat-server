# Aspen on AWS

An [OpenTofu](https://opentofu.org) configuration for one whole deployment in one AWS region.

**This is a prototype.** `tofu validate` accepts it and `tofu test` plans it against made-up
providers, and the images it runs have been built and started. It has never been applied to an
AWS account, so expect to fix things the first time.

## What it makes

| Part of the deployment | On AWS |
| --- | --- |
| API servers, private workers | Fargate tasks behind an Application Load Balancer, which holds the certificate |
| Voice servers | One EC2 machine each, with an Elastic IP for media; signalling through the load balancer |
| PostgreSQL | RDS, reached over TLS with its certificate checked |
| Valkey | ElastiCache, with a password, TLS, and no eviction |
| NATS | One Fargate task, with a user for the API servers and one for each voice server |
| Object storage | A private S3 bucket; uploads go to it directly |
| The storage's read path | CloudFront at `media.<domain>`, which may read the four public prefixes only |
| `aspen.toml`, `voice_server.toml`, NATS's configuration | Written from [`templates/`](templates) into Secrets Manager |
| Migrations, registering voice servers | A task run during each `tofu apply`, before servers are replaced |

Nothing but the load balancer (80 and 443) and the voice servers' media and file transfer ports
is open to the internet.

## Before you start

- A domain for the deployment, in a Route 53 hosted zone. **Choose it for good**
  ([Choosing `public_url`](../../docs/operators/installing/3-configuring.md#choosing-public_url)).
- OpenTofu 1.8 or later, the AWS command line with credentials, Docker, and Python 3, on the
  machine you deploy from.
- Somewhere safe for the state. It holds every password this configuration makes. Use a
  [remote backend](https://opentofu.org/docs/language/settings/backends/s3/) and
  [state encryption](https://opentofu.org/docs/language/state/encryption/).

## The first deployment

1. Describe the deployment:

   ```
   cd deploy/aws
   cp terraform.tfvars.example terraform.tfvars
   ```

   Edit it. [`variables.tf`](variables.tf) describes every variable.

2. Make everything but the servers. With no `image_tag`, no server is started:

   ```
   tofu init
   tofu apply
   ```

3. Build and push the images, from the repository root:

   ```
   TAG=$(git rev-parse --short HEAD)
   API=$(tofu -chdir=deploy/aws output -json image_repositories | jq -r .api)
   VOICE=$(tofu -chdir=deploy/aws output -json image_repositories | jq -r .voice)
   aws ecr get-login-password | docker login --username AWS --password-stdin "${API%%/*}"
   docker build -f deploy/docker/Dockerfile --target api -t "$API:$TAG" . && docker push "$API:$TAG"
   docker build -f deploy/docker/Dockerfile --target voice -t "$VOICE:$TAG" . && docker push "$VOICE:$TAG"
   ```

4. Set `image_tag` in `terraform.tfvars` to the tag, and apply again:

   ```
   tofu apply
   ```

   This creates the tables, registers the voice servers, and then starts every server.

5. Create your account in the web client, then make it the first administrator
   ([Step 7](../../docs/operators/installing/7-first-administrator.md)):

   ```
   $(tofu output -raw run_task) aspen-chat-server admin grant <username>
   ```

6. Check [the storage's read path](../../docs/operators/installing/storage-read-path.md#checking-it)
   from outside. `BASE` is `https://media.<domain>`.

## Operator commands

`tofu output -raw run_task` prints a command that runs whatever follows it as a task of its own,
with the deployment's `aspen.toml`, and prints what it logged:

```
$(tofu output -raw run_task) aspen-chat-server settings show
$(tofu output -raw run_task) aspen-chat-server invites create
```

## Upgrading

Build and push the images under a new tag (step 3), set `image_tag` to it, and `tofu apply`. The
apply follows [Upgrading](../../docs/operators/installing/upgrading.md):

1. It runs `aspen-migrate up`. If that fails the apply stops, and the old servers keep running.
2. It replaces the API servers, new ones taking requests before old ones stop.
3. It restarts each voice server. Its calls end, and their clients rejoin on their own.

## Changing things

| To | Do |
| --- | --- |
| Change a setting this configuration does not write | Add it to `server_environment` as its environment variable. Settings that are lists cannot be given that way. |
| Run more API servers, or private workers | Raise `api.count` or `workers.count`. |
| Add a voice server | Add it to `voice_servers`. Its id is made for it. |
| Remove a voice server | Run `voice-servers remove <name>` (above) first, then take it out of `voice_servers`. |
| Move a voice server to the current machine image | `tofu apply -replace='aws_instance.voice["<name>"]'`. It keeps its address. |
| Change a voice server's capacity | Change it in `voice_servers`. A capacity set in the dashboard is overwritten at the next release. |

## Moving from one machine

A deployment made with [`../aws-single`](../aws-single/README.md) moves here with its accounts,
messages, sign-ins, and address. Its storage is kept, not copied: this deployment takes the
bucket and its read path over. Only the database is copied.

**Not rehearsed on AWS.** The copy itself has been run between local stand-ins of the two
deployments. The steps around it (taking the storage over, the name changing hands) have only
been planned. Read each plan before approving it.

| Step | The old deployment | Takes |
| --- | --- | --- |
| 1 to 3 | Serves as usual | As long as you like |
| 4 | Down | The copy, a release, and up to five minutes for the name to follow |
| 5 | Gone | |

1. In `deploy/aws-single`, apply this release of the configuration to the old deployment. It
   adds the task that exports the database.

2. In `deploy/aws`, write `terraform.tfvars`. Give it a `name` the old deployment does not
   have, and no voice server called `voice`. Leave out `region`, `domain`, and
   `route53_zone_id`, which the next command takes from the old deployment. Then:

   ```
   ./migrate_from_single.py prepare
   tofu init
   tofu apply
   ```

   The plan imports two things, the bucket and the CloudFront distribution, and changes the
   distribution in place. **Stop if it would replace or destroy either.** Everything else is
   new. `domain` still points at the old deployment.

3. Build and push the images to this deployment's repositories
   ([step 3 above](#the-first-deployment)). Do not set `image_tag` yet: the first release must
   find the old deployment's database, not make an empty one.

4. Move. This stops the old deployment:

   ```
   ./migrate_from_single.py copy
   ```

   It stops the old servers, dumps the database into the bucket, and restores it here. Then
   set `image_tag` in `terraform.tfvars` and release:

   ```
   tofu apply
   ```

   The release brings the database up to this release, registers the voice servers, starts
   every server, and points `domain` here. People stay signed in. Calls in progress ended when
   the old servers stopped.

5. When the deployment answers at its address, let go of the old one:

   ```
   ./migrate_from_single.py finish
   tofu -chdir=../aws-single destroy
   ```

   `finish` removes the old voice server's registration and has the old configuration forget
   what this one now uses, so destroying it leaves the storage alone. The old data volume and
   its snapshots are kept as the old database; delete them yourself once you are sure.

   Keep `migration.auto.tfvars.json`. Its `media_bucket` is this deployment's bucket from now
   on.

The dump is deleted from the bucket after seven days. It is never served: the read path has no
`migration/` prefix.

### Going back

Before `finish`, the old deployment is whole. **Whatever was written here since the copy is
lost.**

1. Here, have this configuration forget the storage, so destroying it later leaves it alone:

   ```
   tofu state rm module.media.aws_s3_bucket.media module.media.aws_cloudfront_distribution.media
   ```

2. In `deploy/aws-single`, `tofu apply`. It starts the old servers again and takes back the
   name and the read path.

3. Here, `tofu destroy`.

## What it does not do yet

- **IPv6.** Everything is IPv4. Voice servers carry no media over IPv6.
- **TLS to NATS.** NATS is reached in plaintext inside the private network, and both servers
  warn about it at startup.
- **Metrics.** Both servers' Prometheus endpoints stay on loopback, unscraped.
- **More than one NATS.** While its one task is replaced, no event is delivered.
- **Voice servers elsewhere.** They are in the deployment's region.
- **Backups of the bucket.** RDS keeps seven days of backups. The bucket has no versioning or
  replication ([Backups](../../docs/operators/backups.md)).
- **Scaling by itself.** The counts are what the variables say.

## Checking the configuration

```
tofu fmt -check -recursive
tofu validate
tofu test
```

`tofu test` needs no AWS account. [`tests/plan.tftest.hcl`](tests/plan.tftest.hcl) checks what a
mistake would open or break silently: the ports open to the internet, the proxies the servers
believe, the read path's prefixes, and Valkey's eviction.
