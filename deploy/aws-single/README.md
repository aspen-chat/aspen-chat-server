# Aspen on one AWS machine

An [OpenTofu](https://opentofu.org) configuration for a small deployment that costs little: a
few dozen people, on one machine, running [the box](../box/README.md). [`../aws`](../aws/README.md)
is the one for a deployment that must stay up and grow.

**This is a prototype.** `tofu validate` accepts it and `tofu test` plans it against made-up
providers, and the box it runs has been started, upgraded, and exported on stand-ins for a
machine. It has never been applied to an AWS account, so expect to fix things the first time.

## What it makes

| Part of the deployment | Here |
| --- | --- |
| Everything but the files | [The box](../box/README.md), on one Ubuntu machine: Caddy, both servers, PostgreSQL, Valkey, and NATS |
| The box's directory | A volume of its own, which outlives the machine, snapshotted daily |
| Object storage and its read path | A private S3 bucket, and CloudFront at `media.<domain>`, as in the larger configuration |
| Addresses | An Elastic IP and an IPv6 address, both kept when the machine is replaced |

The deployment is at `<domain>` and its voice server at `voice.<domain>`, over IPv4 and IPv6.
The server reaches the bucket as the machine's role, with no key pair. No SSH port is open:
commands reach the machine through Session Manager.

## What you give up

- **It is down whenever its one machine is**: when the machine is replaced, and when AWS has
  trouble with it or its zone. A release does not stop it (the box starts the new API server
  before it stops the old), but calls end and rejoin.
- **The backups are a day apart.** A lost volume loses what was said since the last snapshot.
- **It does not grow.** A bigger `instance_type` is the only step up. Past that,
  [move to the larger configuration](../aws/README.md#moving-a-box-here), which copies the
  database and keeps everything else, in an outage of some minutes.
- **PostgreSQL is yours to upgrade** between major versions.

## Before you start

- A domain for the deployment, in a Route 53 hosted zone. **Choose it for good**
  ([Choosing `public_url`](../../docs/operators/installing/3-configuring.md#choosing-public_url)).
- OpenTofu 1.8 or later, the AWS command line with credentials, and Python 3, on the machine you
  deploy from.
- Somewhere safe for the state, as for [the larger configuration](../aws/README.md#before-you-start).
  It holds no password of the deployment's: the box makes those, on the machine.

## The first deployment

1. Describe the deployment:

   ```
   cd deploy/aws-single
   cp terraform.tfvars.example terraform.tfvars
   ```

   Edit it. [`variables.tf`](variables.tf) describes every variable. `image_tag` names the
   release to run.

2. Make it:

   ```
   tofu init
   tofu apply
   ```

   This makes the machine, then makes the box on it (`aspen-box init`, with the bucket and the
   machine's addresses) and starts it.

3. Create your account in the web client, then make it the first administrator
   ([Step 7](../../docs/operators/installing/7-first-administrator.md)):

   ```
   $(tofu output -raw on_machine) aspen-box run -- aspen-chat-server admin grant <username>
   ```

4. Measure the voice server's capacity, set it under `voice_servers` in box.json, and `aspen-box
   up` ([Changing things](#changing-things)):

   ```
   $(tofu output -raw on_machine) aspen-box capacity
   ```

5. Check the deployment, and [the storage's read path](../../docs/operators/installing/storage-read-path.md#checking-it)
   from outside (`BASE` is `https://media.<domain>`):

   ```
   $(tofu output -raw on_machine) aspen-box doctor
   ```

## On the machine

`tofu output -raw on_machine` prints a command that runs whatever follows it on the machine,
as root, and prints what it wrote. The box is in `/data/aspen`, and `aspen-box` there works on
it, as [the box's guide](../box/README.md#day-to-day) describes:

```
$(tofu output -raw on_machine) aspen-box run -- aspen-chat-server settings show
$(tofu output -raw on_machine) aspen-box status
```

For a shell: `aws ssm start-session --target $(tofu output -raw instance_id)`.

## Upgrading

Set `image_tag` to the new release, and `tofu apply`. It runs `aspen-box upgrade` on the
machine: the new API server starts beside the old one, which stops once the new one answers,
and calls in progress end and rejoin.

## Changing things

The variables decide the machine and the release. **Everything else is the box's, in
`/data/aspen/box.json`**: edit it in a shell on the machine, then `aspen-box up`, as the box's
guide says. `domain` and the bucket cannot change, and `acme_email` is given to the box only
when it is made.

| To | Do |
| --- | --- |
| Send mail | Give box.json an `email` ([Email](../../docs/operators/installing/3-configuring.md#email)). |
| Use a bigger machine | Change `instance_type`. The machine stops and starts. |
| Move to the current Ubuntu image | `tofu apply -replace=aws_instance.this`. It keeps its addresses and its data, and the release that follows starts the box again. |
| Grow the data volume | Raise `data_volume_gb`, then `resize2fs` its device on the machine. |
| Add a voice server on another machine | As for [any box](../box/README.md#voice-servers-on-other-machines). Open TCP 4222 in the security group, to that machine alone. |

## Backups

- **The box**: a snapshot of the data volume each day, `backup_retention_days` of them kept,
  holding the database, box.json with every password, and the certificates. To restore, make a
  volume from a snapshot in the machine's zone, stop the machine, attach the new volume in the
  old one's place, and start it. Bring the new volume into the state in place of the old (`tofu
  state rm`, `tofu import`). This has not been rehearsed.
- `aspen-box backup` dumps the database into `/data/aspen/backups`, on the same volume.
- **The bucket** has no versioning or replication ([Backups](../../docs/operators/backups.md)).
- `tofu destroy` refuses to delete the data volume.

## Checking the configuration

```
tofu fmt -check -recursive
tofu validate
tofu test
```
