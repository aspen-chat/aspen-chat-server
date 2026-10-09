# The web client

A deployment is one origin, `public_url`. Every API server serves the web client beside the API (`api::web_client`), and the web client calls the API at the origin it was loaded from.

## Pages

- [Serving](serving.md): starting up, which paths the server owns, how files and the page are served and cached, and CORS.
- [Security headers](security-headers.md): the headers on every file and page, the Content Security Policy, and the desktop app's policy.
- [Open Graph previews](open-graph.md): the page's tags, what a page and an invite page preview as, their rate limits, and the revocation checklist.

Why things are the way they are: [design notes](design-notes.md).

## Key files

| Part | Where |
| --- | --- |
| Serving and the startup check | `api::web_client` (`web_client::check`, `web_client::files`) |
| Security headers | `web_client::security_headers`, `content_security_policy` |
| API response headers | `api_response_headers` in `server/api/src/lib.rs` |
| CORS | `api::cors_layer` |
| What a page previews as | `app::open_graph` |
| The tags' template | `server/templates/web_client/open_graph.html` |
| The desktop app's policy | `DESKTOP_POLICY` in `client/packages/app/vite.config.ts` |
| Settings | `public_url`, `[web_client] dir` |
