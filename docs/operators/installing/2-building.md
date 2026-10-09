# Step 2: Building

## The servers

```
cargo build --release -p aspen-chat-server -p aspen-migrate -p voice_server
```

The binaries land in `target/release/`.

| Binary | Needs to build or run |
| --- | --- |
| `aspen-chat-server` | OpenSSL 3 (below), for passkeys and PostgreSQL's TLS. `ffmpeg` and `ffprobe` on `PATH` to show videos inline with a poster (below). |
| `voice_server` | OpenSSL 3 (below), for its calls' encryption. A C++ compiler, `make`, and Python 3, to build mediasoup's C++ worker. |

Both servers link the system's OpenSSL:

- Building them needs OpenSSL 3's development files and `pkg-config`.
- Running them needs OpenSSL 3's libraries (`libssl3`).

Without `ffmpeg` and `ffprobe`, the API server shows pictures inline and offers videos for
download. See [`[media.previews]`](../configuration/previews.md#mediapreviews).

### 64-bit ARM

For 64-bit ARM, such as a Raspberry Pi 5, `scripts/cross_aarch64.py` builds everything on an
x86-64 Linux machine. The binaries need only `libssl3` on the Pi.

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
