# Step 1: The services

Aspen needs four services. `docker-compose.yaml` in the repository starts all four for development,
with passwords written into it. It is not a production setup.

| Service        | What it needs                                                                                                                            |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| PostgreSQL     | 18 or later: the migrations use its `uuidv7()`. They create the `pg_trgm` and `btree_gin` extensions, which the database's owner may do. |
| NATS           | 2.10 or later, with JetStream on (`--jetstream`) and a token (`--auth <token>`). Memory for its streams: see [NATS](#nats).              |
| Valkey         | Or anything that speaks the Redis protocol. A memory limit and no eviction: see [Valkey](#valkey).                                       |
| Object storage | Anything that speaks S3: SeaweedFS, Garage, MinIO, or AWS S3. See [Object storage](#object-storage).                                     |

None of the four belongs on the internet: see [Network exposure](network-exposure.md).

## NATS

Aspen creates the two NATS streams it needs, both in memory: the last minute of events, and the
voice servers' reports until an API server has applied them.

- NATS sets memory aside for a stream's whole size when the stream is made. It may set aside three
  quarters of the machine's memory, or `max_memory_store` in its configuration.
- Unless told otherwise Aspen asks for 1 GiB and 128 MiB, which a machine with 2 GiB allows.
- To have Aspen size the streams to everything NATS has, turn on NATS's monitoring port and name it
  in `aspen.toml`: see [NATS memory](../configuration/services.md#nats-memory).
- To choose the sizes yourself, see
  [the deployment settings](../configuration/deployment-settings.md#nats-memory).

## Valkey

Give Valkey a memory limit and tell it never to evict. In `valkey.conf`:

```
maxmemory 512mb
maxmemory-policy noeviction
```

or on the command line, `--maxmemory 512mb --maxmemory-policy noeviction`.

**Do not set an eviction policy.** It would drop rate limit counts (letting password guesses
through) or sign-ins in progress, at random.

- Everything Aspen keeps in Valkey expires on its own.
- A full Valkey refuses writes, and Aspen treats it as unreachable. Sign-in, password reset,
  registration, and invite endpoints then answer `serverBusy` until it has room.
- Half a gigabyte is plenty for most deployments. Aspen limits how fast strangers can start what it
  stores there (password resets, passkey ceremonies, device links), for everyone together.
- Watch `used_memory` in `INFO memory`, and raise the limit if it approaches it.

## Object storage

The storage needs:

- A bucket.
- A key pair that may read, write, delete, and list in it.
- The S3 API, reachable by clients. They upload to presigned URLs, so it must allow your
  deployment's origin and the apps' by CORS.
- An anonymous read path for downloads: a website endpoint, or a CDN in front of the bucket. It
  allows anonymous reads of objects and nothing else, from any origin by CORS.

See [`[media.s3]`](../configuration/media.md#medias3), and
[The storage's read path](storage-read-path.md).

---

Next: [Step 2: Building](2-building.md)
