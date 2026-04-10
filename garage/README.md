# Garage Bootstrap

*** THESE INSTRUCTIONS WILL NOT PRODUCE A SECURE ENVIRONMENT, DO NOT USE THEM FOR PRODUCTION. ***

The `dxflrs/garage` image is built from `scratch`, so it does not include `/bin/sh`.
For that reason, we do not use a shell-based `garage-init` container.

After `docker compose up -d`, run the following commands manually:

```bash
docker compose exec garage /garage -c /etc/garage.toml status
```

Copy the node id from the status output (64 hex chars), then:

```bash
docker compose exec garage /garage -c /etc/garage.toml layout assign <NODE_ID> -z dc1 -c 50GB
docker compose exec garage /garage -c /etc/garage.toml layout apply --version 1
docker compose exec garage /garage -c /etc/garage.toml key create aspen
```

Note your Key ID and Secret key, you'll use them to populate `aspen.toml`

```bash
docker compose exec garage /garage -c /etc/garage.toml bucket create aspen-media
docker compose exec garage /garage -c /etc/garage.toml bucket allow --read --write aspen-media --key aspen
```
