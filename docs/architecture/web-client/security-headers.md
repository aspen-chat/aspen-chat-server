# Security headers

Code: `web_client::security_headers`, `content_security_policy`, `api_response_headers` in `server/api/src/lib.rs`.

## Headers on every file and page

Every file and page of the web client carries the same headers:

| Header | Value |
| --- | --- |
| `X-Content-Type-Options` | `nosniff` |
| `Referrer-Policy` | `no-referrer`. The video players' frames ask for their own, since some providers refuse a frame that names no page |
| `X-Frame-Options` | `DENY` |
| `Cross-Origin-Opener-Policy` | `same-origin`, so neither a page that opened the web client nor one it opens holds a handle on its window |
| `Strict-Transport-Security` | Over an `https` `public_url` only: `max-age=31536000`, without `includeSubDomains`, since other names under the deployment's may be served otherwise |
| `Content-Security-Policy` | Built from the configuration (`content_security_policy`); see below |

## The Content Security Policy

| What | Allowed from |
| --- | --- |
| Scripts | Only the web client's own (`script-src 'self'`), with `'wasm-unsafe-eval'` for the QR code reader's WebAssembly |
| Styles | Inline styles allowed. React Aria and the emoji picker write `<style>` elements and style attributes; React's own style props are set through the DOM |
| Fonts | Itself |
| Framing the page | Nothing (`frame-ancestors 'none'`) |
| Pictures and videos | The deployment, its storage (`[media.s3] public_base_url`), `data:` and `blob:` (pictures being cropped, the chime), and any `https:` address |
| Requests | The deployment and its own WebSocket, its storage, its upload endpoint (`public_endpoint`, or `endpoint`), and any `https:` or `wss:` address |
| Frames | The deployment's own and any `https:` page |

- The `https:` sources are there because the web client holds sessions on other deployments, whose APIs, storage, voice servers, and plugin views may be anywhere.
- The client itself loads pictures only from deployments' storage, never from an address a message names (see the client's [message rendering](../../../client/docs/architecture/message-rendering/attachments.md#inline-pictures)).
- An `http` `public_url`, a development deployment, also allows `http:` and `ws:` there, as its storage and voice servers use.

The passkey page and the unsubscribe page have policies of their own, with `nosniff`.

## API responses

Every answer under `/api/v1` carries:

- `X-Content-Type-Options: nosniff`.
- `Cache-Control: no-store`, unless its handler chose its own caching (a federation icon, a plugin view's file). So no shared cache keeps what one user was answered.

This is `api_response_headers` in `server/api/src/lib.rs`.

## The desktop app's policy

The desktop app loads the web client's files from disk, where no header can carry a policy. So `pnpm build:desktop` writes one into the page as a `<meta>` (`DESKTOP_POLICY` in `client/packages/app/vite.config.ts`).

It is the same policy, except that nothing but the app's scripts, styles, and fonts may come from a file. So a `file:` address (another machine's share, on Windows) is never loaded as a picture, a video, or a request.
