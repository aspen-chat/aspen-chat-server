# Installing Aspen

Follow these steps in order to set up a deployment, from the services it needs to its first
administrator.

## Steps

1. [The services](1-services.md): PostgreSQL, NATS, Valkey, and object storage.
2. [Building](2-building.md): the servers, the migration tool, and the web client.
3. [Configuring and migrating](3-configuring.md): `aspen.toml` and creating the tables.
4. [Starting the API server](4-api-server.md): TLS, reverse proxies, and private workers.
5. [Serving the web client](5-web-client.md): proxying, headers, link previews, and releases.
6. [Voice servers](6-voice-servers.md): capacity, registering, NATS users, and ports.
7. [The first administrator](7-first-administrator.md): granting the top role, and invite-only
   deployments.
8. [Checking it works](8-checking.md): the smoke test and metrics.

## Securing and maintaining

- [Network exposure](network-exposure.md): what must be reachable, and what must not.
- [The storage's read path](storage-read-path.md): making downloads public without exposing the
  bucket, and checking it.
- [Upgrading](upgrading.md): moving to a new release.
