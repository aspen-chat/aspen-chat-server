# Step 1: The services

Aspen needs four services. `docker-compose.yaml` in the repository starts all four for
development, with passwords written into it. It is not a production setup.

| Service | What it needs |
| --- | --- |
| PostgreSQL | Any supported version. The migrations create the `pg_trgm` extension, which the database's owner may do on PostgreSQL 13 and later. |
| NATS | 2.10 or later, with JetStream on (`--jetstream`) and a token (`--auth <token>`). |
| Valkey | Or anything that speaks the Redis protocol. A memory limit and no eviction: see [Valkey](#valkey). |
| Object storage | Anything that speaks S3: SeaweedFS, Garage, MinIO, or AWS S3. See [Object storage](#object-storage). |

Aspen creates the NATS stream it needs, and keeps only the last minute of events, in memory.

None of the four belongs on the internet: see [Network exposure](network-exposure.md).

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
- Half a gigabyte is plenty for most deployments. Aspen limits how fast strangers can start what
  it stores there (password resets, passkey ceremonies, device links), for everyone together.
- Watch `used_memory` in `INFO memory`, and raise the limit if it approaches it.

## Object storage

The storage needs:

- A bucket.
- A key pair that may read, write, delete, and list in it.
- The S3 API, reachable by clients. They upload to presigned URLs, so it must allow your
  deployment's origin and the apps' by CORS.
- An anonymous read path for downloads: a website endpoint, or a CDN in front of the bucket. It
  allows anonymous reads of objects and nothing else.

See [`[media.s3]`](../configuration/media.md#medias3), and
[The storage's read path](storage-read-path.md).

---

Next: [Step 2: Building](2-building.md)
