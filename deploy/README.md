# Deploying from files

What is here turns the steps of the [operator guide](../docs/operators/installing/index.md) into
files a tool applies.

| Directory | What it holds |
| --- | --- |
| [`docker/`](docker/Dockerfile) | The container images of both servers. |
| [`box/`](box/README.md) | A whole deployment on one Linux machine, anyone's: `aspen-box` writes its configuration, starts it, upgrades it without stopping the API, and checks it. |
| [`templates/`](templates/README.md) | `aspen.toml`, `voice_server.toml`, and NATS's configuration, as everything here writes them. |
| [`aws/`](aws/README.md) | A whole deployment on AWS, for [OpenTofu](https://opentofu.org): managed services, several API servers, a machine for each voice server. |
| [`aws-single/`](aws-single/README.md) | A box on one AWS machine, with its files in S3, which costs a fraction of that and is down whenever the machine is. |

The two AWS configurations are prototypes: planned, never applied. They share the storage, in
`aws/modules/media`. A box that outgrows its machine, on AWS or anywhere else,
[moves to the larger configuration](aws/README.md#moving-a-box-here).

## The images

They are published at `ghcr.io/aspen-chat/aspen-api` and `ghcr.io/aspen-chat/aspen-voice`, for
x86-64 and 64-bit ARM: `main` and each commit's first twelve characters for every change to
`main`, and each release's tag (`.github/workflows/images.yml`). Everything here runs those,
by tag, unless told other images.

To build them yourself, from the repository root, with Docker's BuildKit (`docker buildx`, the
default builder of current Docker):

```
docker build -f deploy/docker/Dockerfile --target api -t aspen-api . \
  --build-arg ASPEN_COMMIT=$(git rev-parse --short=12 HEAD)
docker build -f deploy/docker/Dockerfile --target voice -t aspen-voice .
```

`ASPEN_COMMIT` is the commit About Aspen names; add `--build-arg ASPEN_MODIFIED=true` when the
checkout has changes of its own.

| Image | Holds | Starts |
| --- | --- | --- |
| `api` | `aspen-chat-server`, `aspen-migrate`, the web client, `curl` | `aspen-chat-server --no-https --port 8080`, for a load balancer or reverse proxy that terminates TLS |
| `voice` | `voice_server`, `curl` | `voice_server` |

Neither holds `ffmpeg`, so the `api` image offers videos for download without a poster. To take
posters, build an image on it that installs `ffmpeg`, and set `[media.previews] ffmpeg` and
`ffprobe` to it ([`[media.previews]`](../docs/operators/configuration/previews.md#mediapreviews)).

- Both run as the user `aspen` (10001) in `/etc/aspen`, and read their configuration there:
  mount `aspen.toml` or `voice_server.toml` into it, or have the container write it before it
  starts the server, as [`aws/servers.tf`](aws/servers.tf) does.
- The `api` image sets `ASPEN_WEB_CLIENT__DIR` to the web client it holds, so `aspen.toml` needs
  no `[web_client]`.
- Every other setting can also be given in the container's environment
  ([How settings are read](../docs/operators/configuration/index.md#how-settings-are-read)).
- The operator commands run in the `api` image: `docker run … aspen-api aspen-chat-server admin
  grant <username>`, with the same configuration.
- An image is built for the architecture of the machine that builds it.

A new `api` image holds only its own web client's files. People with the web client open during a
release fetch some parts later, from the release they started with
([Releasing a new web client](../docs/operators/installing/5-web-client.md#releasing-a-new-web-client)),
and those are gone once the old containers are. Those pages reload when they find it
(`client/docs/architecture/new-release.md`).
