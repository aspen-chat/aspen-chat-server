# Aspen on AWS

An [OpenTofu](https://opentofu.org) configuration for one whole deployment in one AWS region.

**This is a prototype.** `tofu validate` accepts it and `tofu test` plans it against made-up
providers, and the images it runs have been built and started. It has never been applied to an AWS
account, so expect to fix things the first time.

## What it makes

| Part of the deployment                                  | On AWS                                                                                                                  |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| API servers, private workers                            | Fargate tasks behind an Application Load Balancer, which holds the certificate                                          |
| Voice servers                                           | One EC2 machine each, with an Elastic IP and an IPv6 address of its own for media; signalling through the load balancer |
| PostgreSQL                                              | RDS, reached over TLS with its certificate checked                                                                      |
| Valkey                                                  | ElastiCache, with a password, TLS, and no eviction                                                                      |
| NATS                                                    | One Fargate task, with a user for the API servers and one for each voice server                                         |
| Object storage                                          | A private S3 bucket, which the servers reach as their task role; uploads go to it directly                              |
| The storage's read path                                 | CloudFront at `media.<domain>`, which may read the four public prefixes only                                            |
| `aspen.toml`, `voice_server.toml`, NATS's configuration | Written from [`templates/`](templates) into Secrets Manager, each readable by its own kind of task alone                |
| Migrations, registering voice servers                   | A task run during each `tofu apply`, before servers are replaced                                                        |

Nothing but the load balancer (80 and 443) and the voice servers' media and file transfer ports is
open to the internet. All of them, and the read path, answer over IPv4 and IPv6, and the servers
reach the internet over both. Each voice server's machine refuses the voice server its instance
metadata service ([`voice-imds.service`](machine/voice-imds.service)).

The servers run the published images, `ghcr.io/aspen-chat/aspen-api` and `aspen-voice`, built for
x86-64 and 64-bit ARM. To run images of your own, build them from
[`deploy/docker/Dockerfile`](../docker/Dockerfile), push them where ECS can pull them without
signing in (a public repository) or to ECR, and name them in `image_repositories`.

## Before you start

- A domain for the deployment, in a Route 53 hosted zone. **Choose it for good**
  ([Choosing `public_url`](../../docs/operators/installing/3-configuring.md#choosing-public_url)).
- OpenTofu 1.8 or later, the AWS command line with credentials, and Python 3, on the machine you
  deploy from.
- Somewhere safe for the state. It holds every password this configuration makes. Use a
  [remote backend](https://opentofu.org/docs/language/settings/backends/s3/) and
  [state encryption](https://opentofu.org/docs/language/state/encryption/).

## The first deployment

1. Describe the deployment:

   ```
   cd deploy/aws
   cp terraform.tfvars.example terraform.tfvars
   ```

   Edit it. [`variables.tf`](variables.tf) describes every variable. `image_tag` names the release
   to run.

2. Make it:

   ```
   tofu init
   tofu apply
   ```

   This makes everything, creates the tables, registers the voice servers, and then starts every
   server.

   It also turns on IPv6 for every Fargate task in the account and region that does not say
   otherwise (`dualStackIPv6`, an account setting), which tasks elsewhere in a dual-stack subnet
   then take too.

3. Create your account in the web client, then make it the first administrator
   ([Step 7](../../docs/operators/installing/7-first-administrator.md)):

   ```
   $(tofu output -raw run_task) aspen-chat-server admin grant <username>
   ```

4. Check [the storage's read path](../../docs/operators/installing/storage-read-path.md#checking-it)
   from outside. `BASE` is `https://media.<domain>`.

## Operator commands

`tofu output -raw run_task` prints a command that runs whatever follows it as a task of its own,
with the deployment's `aspen.toml`, and prints what it logged:

```
$(tofu output -raw run_task) aspen-chat-server settings show
$(tofu output -raw run_task) aspen-chat-server invites create
```

## Upgrading

Set `image_tag` to the new release, and `tofu apply`. The apply follows
[Upgrading](../../docs/operators/installing/upgrading.md):

1. It runs `aspen-migrate up`. If that fails the apply stops, and the old servers keep running.
2. It replaces the API servers, new ones taking requests before old ones stop.
3. It restarts each voice server. Its calls end, and their clients rejoin on their own.

## Changing things

| To                                                 | Do                                                                                                                                                                             |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Change a setting this configuration does not write | Add it to `server_environment` as its environment variable ([How settings are read](../../docs/operators/configuration/index.md#how-settings-are-read)): lists and tables too. |
| Run more API servers, or private workers           | Raise `api.count` or `workers.count`.                                                                                                                                          |
| Add a voice server                                 | Add it to `voice_servers`. Its id is made for it.                                                                                                                              |
| Remove a voice server                              | Run `voice-servers remove <name>` (above) first, then take it out of `voice_servers`.                                                                                          |
| Move a voice server to the current machine image   | `tofu apply -replace='aws_instance.voice["<name>"]'`. It keeps its address.                                                                                                    |
| Change a voice server's capacity                   | Change it in `voice_servers`. A capacity set in the dashboard is overwritten at the next release.                                                                              |

## Moving Ultra Lite here

An [Ultra Lite](../../docs/operators/ultra-lite/index.md) deployment (`aspen-ul`) moves here with
its accounts, messages, sign-ins, and address. Its database is copied, and its files are copied into
this deployment's bucket, each with the type it is served with.

**Not rehearsed on AWS.** The copy itself has been run between local stand-ins of the two
deployments (`scripts/check_ul_export.py`). The steps around it (the name changing hands) have only
been planned. Read each plan before approving it.

| Step    | The old deployment | Takes                                                              |
| ------- | ------------------ | ------------------------------------------------------------------ |
| 1 and 2 | Serves as usual    | As long as you like                                                |
| 3       | Down               | The copy, a release, and up to five minutes for the name to follow |
| 4       | Gone               |                                                                    |

1. Its domain must be in a Route 53 hosted zone first. Make the zone, write the records the
   deployment has now into it (its `A` and `AAAA` records, for the domain and `media.` under it),
   move the domain's name servers to it at your registrar, and wait until `dig NS <domain>` names
   Route 53's.

2. In `deploy/aws`, write `terraform.tfvars`, without `image_tag`: with none, no server starts, and
   the first release must find the old deployment's database, not make an empty one. Give no voice
   server the name `aspen-ul`, which Ultra Lite's own has. Then:

   ```
   ./migrate.py prepare --domain chat.example.org
   tofu apply
   ```

   `domain` still points at the old deployment.

3. Move. This stops the old deployment. Stop it and export it on its machine, which writes
   everything into a directory, with room for every file people have uploaded:

   ```
   sudo systemctl stop aspen-ul
   sudo aspen-ul export ~/aspen-export
   ```

   Copy the directory to the machine you deploy from (`rsync -a`), keeping it where only you can
   read it, since it is every account's data, and:

   ```
   ./migrate.py copy --export ~/aspen-export
   ```

   It copies the database into the bucket and restores it here. Its files are uploaded by
   `aspen-chat-server storage import` from the API image (`--image`, the `main` image by default),
   run in Docker on this machine with the AWS command line's credentials (its environment, or
   `~/.aws`), so each keeps the type it is served with. A deployment already keeping its files in
   object storage has them copied first: `copy` says how, and then takes `--files-copied`. Then set
   `image_tag` in `terraform.tfvars` and release:

   ```
   tofu apply
   ```

   The release brings the database up to this release, registers the voice servers, starts every
   server, and points `domain` and `media.<domain>` here. People stay signed in. Calls in progress
   ended when the old deployment stopped.

4. When the deployment answers at its address, let go of the old one:

   ```
   ./migrate.py finish
   ```

   It removes the registration of Ultra Lite's voice server. Keep Ultra Lite from starting again:
   `sudo systemctl disable aspen-ul`. Keep its data directory and the export until you are sure.

   Keep `migration.auto.tfvars.json`: it holds `domain`.

The dump is deleted from the bucket after seven days. It is never served: the read path has no
`migration/` prefix.

### Going back

Before `finish`, the old deployment is whole. **Whatever was written here since the copy is lost.**
Here, `tofu destroy`, which takes the records with it; write the old deployment's records into the
zone again, and `sudo systemctl start aspen-ul` on its machine.

## What it does not do yet

- **TLS to NATS.** NATS is reached in plaintext inside the private network, and both servers warn
  about it at startup.
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
