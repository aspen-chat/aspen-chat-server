# Aspen

Aspen is a free and open source chat platform for communities. It offers text chat, voice calls,
camera video, and screen and game sharing. Each deployment runs on its own, and users of one
deployment can join communities on others through federation. A deployment can run on a single
spare machine, and its servers can be added to as it grows.

This repository holds the API server, the voice server, and one client app. The app runs in the
browser, on Windows, macOS, and Linux, and on Android and iOS. Aspen is licensed under the
[Mozilla Public License 2.0](LICENSE).

## Documentation

### Running a deployment

- [Operator guide](docs/operators/README.md): start here to run a deployment.
- [Installing](docs/operators/installing/index.md)
- [Configuration](docs/operators/configuration/index.md)
- [Federation](docs/operators/federation/index.md)
- [Plugins](docs/operators/plugins.md)
- [Backups](docs/operators/backups.md)
- [Troubleshooting](docs/operators/troubleshooting/index.md)

### Working on Aspen

- [Server architecture](docs/architecture/): one page or folder per feature.
- [Client README](client/README.md): building and running the client apps.
- [Client architecture](client/docs/architecture/): one page or folder per client feature.
- [Benchmarking](bench/README.md): load-testing a deployment with `aspen-bench`.
- [AGENTS.md](AGENTS.md) and [client/AGENTS.md](client/AGENTS.md): conventions and standards for
  contributors, both human and AI.

### Specifications

- [Federation protocol](spec/federation.md)
- [Push notifications](spec/push.md)
- [Plugins](spec/plugins.md)
