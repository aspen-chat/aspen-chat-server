# Serving

## Starting up

`[web_client] dir` is the built web client (`pnpm build`). The server refuses to start unless it holds (`web_client::check`):

- An `index.html` with a `</head>`.
- `open-graph.png`.

A private worker is the exception (`--private-worker`, `app::context::Role::PrivateWorker`). It opens no listening socket, leaves its event feed idle, and does only the background work every API server shares, so it serves no web client.

The development stack keeps the same shape with the Vite dev server in front. Its `public_url` is Vite's origin, which proxies the server's own paths to the server.

## Paths the server owns

| Path | Answers |
| --- | --- |
| `/api/v1` | The API |
| Every other path under `/api/` | A `404` Problem rather than a page, so a newer client's request is not answered with HTML |
| `/auth/passkey` | The passkey page |
| `/.well-known/aspen` | The federation document |
| `/email/unsubscribe` | The unsubscribe page |
| `/invite/{code}` | The invite page |

## Everything else

Every other path falls to `web_client::files`, a `tower_http` `ServeDir` over the directory:

- A file where there is one.
- The page where there is none, directories included. So `/` is the page.
- A missing file under `/assets/` is a `404` rather than the page. A page of an older release asking for its own script cannot run HTML.

Everything is read from the directory as it is asked for. So a release copied into place is served at once, with no restart.

## Caching

| Files | `Cache-Control` |
| --- | --- |
| Under `/assets/` (the build names them by their contents) | `public, max-age=31536000, immutable` |
| Everything else, the page included | `no-cache`, so a new release reaches browsers on their next load |

## `view-fonts.css`

The build writes `view-fonts.css` at the root: every `@font-face` the app bundles, under that one name.

- Plugins' views link to it to draw in the app's fonts (`spec/plugins.md`, Views).
- Its faces' files are the app's own under `/assets/`.
- A view's sandboxed page has an origin of its own, so it fetches them with CORS, which the deployment allows.

## CORS

CORS allows every origin (`api::cors_layer`). The web client needs none, but these call the API from elsewhere:

- The desktop app's pages (`file:`).
- The mobile apps'.
- Other deployments' web clients.

**Why it is safe:** every request is authenticated by a bearer token rather than a cookie. See [design notes](design-notes.md#cors).
