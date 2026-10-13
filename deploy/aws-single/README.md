# Aspen on one AWS machine

An [OpenTofu](https://opentofu.org) configuration for a small deployment that costs little: a
few dozen people, on one machine. [`../aws`](../aws/README.md) is the one for a deployment that
must stay up and grow.

**This is a prototype.** `tofu validate` accepts it and `tofu test` plans it against made-up
providers, and its containers have been started together with the commands and files it gives
them. It has never been applied to an AWS account, so expect to fix things the first time.

## What it makes

| Part of the deployment | Here |
| --- | --- |
| API server, voice server | One container each, on the machine |
| PostgreSQL, Valkey, NATS | One container each, on the machine, listening on loopback |
| TLS | Caddy, on the machine, with certificates from Let's Encrypt |
| The database's files | A volume of their own, which outlives the machine, snapshotted daily |
| Object storage and its read path | A private S3 bucket, and CloudFront at `media.<domain>`, as in the larger configuration |

The deployment is at `<domain>` and its voice server at `voice.<domain>`.

## What you give up

- **It is down whenever its one machine or its one API server is**: during every release (a
  minute or so), when the machine is replaced, and when AWS has trouble with it or its zone.
- **The backups are a day apart.** A lost volume loses what was said since the last snapshot.
- **It does not grow.** A bigger `instance_type` is the only step up. Past that, move to the
  larger configuration, which means moving the database.
- **PostgreSQL is yours to upgrade** between major versions.

## Before you start

The same as [the larger configuration](../aws/README.md#before-you-start).

## The first deployment

1. Describe the deployment:

   ```
   cd deploy/aws-single
   cp terraform.tfvars.example terraform.tfvars
   ```

   Edit it. [`variables.tf`](variables.tf) describes every variable.

2. Make everything but the two servers:

   ```
   tofu init
   tofu apply
   ```

3. Build and push the images, as in
   [the larger configuration's step 3](../aws/README.md#the-first-deployment), with
   `-chdir=deploy/aws-single`.

4. Set `image_tag` in `terraform.tfvars` to the tag, and apply again. This creates the tables,
   registers the voice server, and starts both servers.

5. Create your account in the web client, then make it the first administrator:

   ```
   $(tofu output -raw run_task) aspen-chat-server admin grant <username>
   ```

6. Measure the voice server's capacity on the machine, and set `voice_capacity` to it. Open a
   shell with `aws ssm start-session --target $(tofu output -raw instance_id)`, then:

   ```
   sudo docker exec $(sudo docker ps -q --filter name=voice) voice_server estimate-capacity
   ```

7. Check [the storage's read path](../../docs/operators/installing/storage-read-path.md#checking-it)
   from outside. `BASE` is `https://media.<domain>`.

## Paying less for the machine

The machine is most of the bill, and it runs all month. Buy a reserved instance for its
`instance_type` in its region, or a compute savings plan, in the AWS console: either is a
billing arrangement that applies to the machine by itself, and this configuration buys neither.

## Operator commands, upgrading

As in the larger configuration: [operator commands](../aws/README.md#operator-commands), and
for a release, push the images under a new tag, set `image_tag`, and `tofu apply`. The
deployment is down from when the old API server stops until the new one has started.

## Backups

- **The database**: a snapshot of the data volume each day, `backup_retention_days` of them
  kept. To restore, make a volume from a snapshot in the machine's zone, stop the machine,
  attach the new volume in the old one's place, and start it. Bring the new volume into the
  state in place of the old (`tofu state rm`, `tofu import`). This has not been rehearsed.
- **The bucket** has no versioning or replication ([Backups](../../docs/operators/backups.md)).
- `tofu destroy` refuses to delete the data volume.

## Changing things

| To | Do |
| --- | --- |
| Change a setting this configuration does not write | Add it to `server_environment` as its environment variable. Settings that are lists cannot be given that way. |
| Use a bigger machine | Change `instance_type`. The machine stops and starts. |
| Move to the current machine image | `tofu apply -replace=aws_instance.this`. It keeps its address and its data. |
| Grow the data volume | Raise `data_volume_gb`, then run `sudo xfs_growfs /data` on the machine. |

## Checking the configuration

```
tofu fmt -check -recursive
tofu validate
tofu test
```
