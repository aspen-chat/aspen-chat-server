# Deploying from files

What is here turns the steps of the [operator guide](../docs/operators/installing/index.md) into
files a tool applies.

| Directory | What it holds |
| --- | --- |
| [`docker/`](docker/Dockerfile) | The container images of both servers. |
| [`aws/`](aws/README.md) | A whole deployment on AWS, for [OpenTofu](https://opentofu.org): managed services, several API servers, a machine for each voice server. |
| [`aws-single/`](aws-single/README.md) | A small deployment on one AWS machine, which costs a fraction of that and is down whenever the machine is. |

Both are prototypes: planned, never applied. They share the images, the configuration
templates in `aws/templates/`, the modules in `aws/modules/`, and `aws/run_task.py`. A deployment
that outgrows one machine [moves to the larger configuration](aws/README.md#moving-from-one-machine).

## The images

Build them from the repository root, with Docker's BuildKit (`docker buildx`, the default builder
of current Docker):

```
docker build -f deploy/docker/Dockerfile --target api   -t aspen-api .
docker build -f deploy/docker/Dockerfile --target voice -t aspen-voice .
```

| Image | Holds | Starts |
| --- | --- | --- |
| `api` | `aspen-chat-server`, `aspen-migrate`, the web client, `ffmpeg` and `ffprobe` | `aspen-chat-server --no-https --port 8080`, for a load balancer or reverse proxy that terminates TLS |
| `voice` | `voice_server` | `voice_server` |

- Both run as the user `aspen` (10001) in `/etc/aspen`, and read their configuration there:
  mount `aspen.toml` or `voice_server.toml` into it, or have the container write it before it
  starts the server, as [`aws/servers.tf`](aws/servers.tf) does.
- The `api` image sets `ASPEN_WEB_CLIENT__DIR` to the web client it holds, so `aspen.toml` needs
  no `[web_client]`.
- Every other setting can also be given in the container's environment
  ([Environment variables](../docs/operators/installing/3-configuring.md#environment-variables)),
  except those that are lists.
- The operator commands run in the `api` image: `docker run … aspen-api aspen-chat-server admin
  grant <username>`, with the same configuration.
- An image is built for the architecture of the machine that builds it.

A new `api` image holds only its own web client's files. People with the web client open during a
release fetch some parts later, from the release they started with
([Releasing a new web client](../docs/operators/installing/5-web-client.md#releasing-a-new-web-client)),
and those are gone once the old containers are.
