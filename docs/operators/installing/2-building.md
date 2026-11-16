# Step 2: Building

## The servers

```
cargo build --release -p aspen-chat-server -p aspen-migrate -p voice_server
```

The binaries land in `target/release/`.

| Binary              | Needs to build or run                                                                                                               |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `aspen-chat-server` | OpenSSL 3 (below), for passkeys and PostgreSQL's TLS. `ffmpeg` and `ffprobe` on `PATH` to show videos inline with a poster (below). |
| `voice_server`      | Nothing beyond what every build needs. Its calls' encryption is AWS-LC, compiled in; it needs no OpenSSL.                           |

Every build needs Rust (the version `rust-toolchain.toml` names, which `rustup` installs), a C
compiler, and `make`, for the C some crates compile in (AWS-LC, ring, jemalloc, and libwebp).

The API server and `aspen-migrate` link the system's OpenSSL and libpq:

- Building them needs OpenSSL 3's and libpq's development files (`libssl-dev`, `libpq-dev`) and
  `pkg-config`.
- Running them needs their libraries (`libssl3`, `libpq5`).

Without `ffmpeg` and `ffprobe`, the API server shows pictures inline and offers videos for download.
See [`[media.previews]`](../configuration/previews.md#mediapreviews).

### 64-bit ARM

For 64-bit ARM, such as a Raspberry Pi 5, `scripts/cross_aarch64.py` builds everything on an x86-64
Linux machine. On the Pi, the API server needs `libssl3` and `libpq5`, and the voice server nothing
beyond the C library.

## The web client

Every API server serves the web client, and will not start without it. Build it:

```
cd client
pnpm install
pnpm build
```

It lands in `client/packages/app/dist/`. Copy that directory to each API server's machine.

---

Previous: [Step 1: The services](1-services.md) · Next:
[Step 3: Configuring and migrating](3-configuring.md)
