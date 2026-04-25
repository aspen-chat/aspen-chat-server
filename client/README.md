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
- `ASPEN_UI` (`widgets` / `quick`, default `widgets`) — selects the UI layer

## Experimental: Qt Quick UI

A parallel Qt Quick (QML) implementation lives in `src/aspen_client/qml_ui/`. It reuses every non-UI layer (`AspenApiClient`, `TaskSpawner`, `ClientState`, `EventStreamClient`, `IconCache`, `LinkPreviewImageCache`, `UserDirectory`) and adds `QObject` controllers, `QAbstractListModel`-backed list models, a `QQuickImageProvider` for cached avatars and link-preview thumbnails, and the QML scene tree.

Opt in by exporting `ASPEN_UI=quick` before launching:

```bash
source .venv/bin/activate
ASPEN_API_BASE_URL=https://127.0.0.1:443 \
ASPEN_VERIFY_TLS=false \
ASPEN_UI=quick \
aspen-client
```

Parity status: login, communities/channels/users panels, message pane (sliding window + paging + previews + composer), reconnect-resync, and shutdown all work end-to-end. The Widgets path remains the default until the Quick path has had at least one full human-validation pass.

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
ASPEN_UI=widgets aspen-client   # smoke: Widgets path still works
ASPEN_UI=quick   aspen-client   # smoke: Quick path boots, login, chat
```
