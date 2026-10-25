# syntax=docker/dockerfile:1
#
# The images a deployment runs, built from the repository root:
#
#   docker build -f deploy/docker/Dockerfile --target api   -t aspen-api . \
#     --build-arg ASPEN_COMMIT=$(git rev-parse --short=12 HEAD)
#   docker build -f deploy/docker/Dockerfile --target voice -t aspen-voice .
#
# `api` holds aspen-chat-server, aspen-migrate, the web client, and the box's tool; `voice` holds
# voice_server.
# Both hold curl, for asking a server's health from inside its container. Neither holds
# configuration: each reads `aspen.toml` or `voice_server.toml` from its working directory,
# /etc/aspen, and `ASPEN_` variables from its environment.

# The oldest ca-certificates either stage may hold: Debian 12's first that no longer trusts the
# roots the compromised eMudhra intermediates chain to (https://emudhra.we-have-a-problem.com/,
# https://bugs.debian.org/1151413). The build stage fetches toolchains and crates over TLS, and
# the runtime stage's trust store is what OpenSSL checks PostgreSQL's certificate against. A
# layer cached from before the package changed fails the build rather than ship.
ARG MIN_CA_CERTIFICATES=20250419~deb12u2

# Debian 12, as the ARM build in CI: its glibc and OpenSSL 3 are what the runtime stages have.
# One stage builds the servers and the web client, because the web client's build needs both
# toolchains: its attributions page lists the servers' crates, which it asks cargo for.
FROM debian:bookworm AS build
ARG MIN_CA_CERTIFICATES
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git build-essential \
        pkg-config libssl-dev libpq-dev python3 python3-pip python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && dpkg --compare-versions "$(dpkg-query -Wf '${Version}' ca-certificates)" \
        ge "$MIN_CA_CERTIFICATES"
COPY --from=node:22-bookworm-slim /usr/local /usr/local
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /src
# The toolchains alone first, so a change to the source does not install them again.
COPY rust-toolchain.toml .
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none \
    && rustup toolchain install \
    && corepack enable pnpm
COPY . .
# Only cargo's downloads are kept between builds. A kept `target/` would be shared by every
# checkout built on the machine, and cargo, which goes by modification times, takes an older
# checkout's sources for already built: the image would then hold another checkout's servers.
# Each server writes its schemas to the repository root, where the web client is generated
# from them.
# jemalloc fixes its page size when compiled. A Raspberry Pi 5's kernel has 16 KiB pages, where
# a jemalloc built for 4 KiB aborts at startup, and 64 KiB pages serve every smaller size.
ARG TARGETARCH
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    if [ "$TARGETARCH" = arm64 ]; then export JEMALLOC_SYS_WITH_LG_PAGE=16; fi \
    && cargo build --locked --release -p aspen-chat-server -p aspen-migrate -p voice_server \
    && mkdir -p /out/bin \
    && cp target/release/aspen-chat-server target/release/aspen-migrate target/release/voice_server /out/bin/ \
    && /out/bin/aspen-chat-server --gen-openapi-schema \
    && /out/bin/voice_server --gen-signal-schema
# The crates' sources, which the attributions page reads their licences from, are in cargo's
# caches. The build has no `.git`, so the commit About Aspen names is handed in: the one built,
# and whether the checkout had changes of its own (`true`).
ARG ASPEN_COMMIT=
ARG ASPEN_MODIFIED=false
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/root/.local/share/pnpm/store \
    cd client \
    && pnpm install --frozen-lockfile \
    && ASPEN_BUILD_COMMIT="$ASPEN_COMMIT" ASPEN_BUILD_MODIFIED="$ASPEN_MODIFIED" \
        pnpm --filter @aspen/app build \
    && cp -r packages/app/dist /out/web

FROM debian:bookworm-slim AS runtime
ARG MIN_CA_CERTIFICATES
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 zlib1g libbrotli1 libzstd1 \
        curl \
    && rm -rf /var/lib/apt/lists/* \
    && dpkg --compare-versions "$(dpkg-query -Wf '${Version}' ca-certificates)" \
        ge "$MIN_CA_CERTIFICATES"

# A user of its own, 10002, not the API server's: a machine running both tells them apart by
# it, and refuses this one the cloud's instance metadata service
# (deploy/aws-single/machine/voice-imds.service).
FROM runtime AS voice
RUN useradd --system --uid 10002 --home-dir /etc/aspen --shell /usr/sbin/nologin aspen-voice \
    && install -d -o aspen-voice -g aspen-voice -m 0700 /etc/aspen
WORKDIR /etc/aspen
COPY --from=build /out/bin/voice_server /usr/local/bin/
USER aspen-voice
CMD ["voice_server"]

FROM runtime AS api
RUN useradd --system --uid 10001 --home-dir /etc/aspen --shell /usr/sbin/nologin aspen \
    && install -d -o aspen -g aspen -m 0700 /etc/aspen
WORKDIR /etc/aspen
# openssl makes the certificates a box opens NATS with to voice servers elsewhere
# (deploy/box/aspen_box/nats_tls.py).
RUN apt-get update \
    && apt-get install -y --no-install-recommends openssl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/bin/aspen-chat-server /out/bin/aspen-migrate /usr/local/bin/
COPY --from=build /out/web /srv/aspen/web
# The box's tool and the templates it fills, for a machine with Docker and no checkout of the
# repository to take them from: `docker run --rm --entrypoint tar <image> -C /usr/share/aspen
# -c . | tar -x` (deploy/box/README.md).
COPY deploy/box /usr/share/aspen/box
COPY deploy/templates /usr/share/aspen/templates
# The environment overrides `aspen.toml`, so whichever one a deployment gives this image, the
# server serves the web client built with it.
ENV ASPEN_WEB_CLIENT__DIR=/srv/aspen/web
# No ffmpeg is published with Aspen, so videos are offered for download without a poster. A
# deployment that wants posters builds an image on this one that adds it, and sets these to it
# (docs/operators/configuration/previews.md).
ENV ASPEN_MEDIA__PREVIEWS__FFMPEG="" ASPEN_MEDIA__PREVIEWS__FFPROBE=""
USER aspen
EXPOSE 8080
# Plain HTTP, for the load balancer or reverse proxy that terminates TLS before it.
CMD ["aspen-chat-server", "--no-https", "--port", "8080"]
