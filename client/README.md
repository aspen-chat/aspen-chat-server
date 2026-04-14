# Aspen Qt Client

Qt for Python desktop client for Aspen server.

## Prerequisites

- Python 3.12+
- Aspen server running locally (or configured URL)

## Install

```bash
cd client
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
```

## Generate API/Event models

First regenerate the server schema files (from repo root):

```bash
cargo run -- --gen-openapi-schema
```

Then regenerate Python models (from this `client` directory):

```bash
source .venv/bin/activate
generate-client
```

## Run the app

```bash
source .venv/bin/activate
ASPEN_API_BASE_URL=https://127.0.0.1:443 \
ASPEN_VERIFY_TLS=false \
aspen-client
```

Optional environment variables:

- `ASPEN_API_BASE_URL` (default `https://127.0.0.1:443`)
- `ASPEN_WS_URL` (default derived from API base URL + `/event_stream`)
- `ASPEN_VERIFY_TLS` (`true`/`false`, default `false`)

## Current MVP features

- Login via `/login`
- Community list via `/user/communities`
- Channel list via `/community/channels`
- Message history via `/channel/messages`
- Send messages via `/message`
- Receive incoming message create events via `/event_stream`

## Validation checks

Server checks (from repo root):

```bash
cargo run -- --gen-openapi-schema
cargo clippy --all-targets --all-features
```

Client checks (from `client`):

```bash
source .venv/bin/activate
generate-client
python -m compileall src/aspen_client
```
