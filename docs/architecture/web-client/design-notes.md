# The web client: design notes

Why the web client is served as it is. The how is in the [main pages](index.md).

## One origin

- **Every API server serves the web client at `public_url`.** So links, passkeys, and the pages the apps open all name one address, and the web client calls the API at the origin it was loaded from.
- **A private worker serves no web client.** It opens no listening socket and does only the shared background work.

## Serving

See [Serving](serving.md).

- **Every path under `/api/` that is not an API route is a `404` Problem, not the page.** So a newer client's request is not answered with HTML.
- **A missing file under `/assets/` is a `404`, not the page.** A page of an older release asking for its own script cannot run HTML.
- **Files under `/assets/` are cached forever; everything else is `no-cache`.** The build names asset files by their contents, and a new release reaches browsers on their next load.
- **Files are read from the directory as they are asked for.** So a release copied into place is served at once, with no restart.

### CORS

- **CORS allows every origin.** The desktop app's pages (`file:`), the mobile apps', and other deployments' web clients call the API from elsewhere. Every request is authenticated by a bearer token rather than a cookie, so no origin gains anything a page could not already do with the token it holds.

## Security headers

See [Security headers](security-headers.md).

- **`Cross-Origin-Opener-Policy: same-origin`.** So neither a page that opened the web client nor one it opens holds a handle on its window.
- **`Strict-Transport-Security` without `includeSubDomains`.** Other names under the deployment's may be served otherwise.
- **The video players' frames ask for their own referrer policy.** Some providers refuse a frame that names no page.
- **Inline styles are allowed.** React Aria and the emoji picker write `<style>` elements and style attributes.
- **`'wasm-unsafe-eval'` is allowed.** The QR code reader is WebAssembly.
- **Any `https:` source is allowed for pictures, requests, and frames.** The web client holds sessions on other deployments, whose APIs, storage, voice servers, and plugin views may be anywhere. The client itself loads pictures only from deployments' storage, never from an address a message names.
- **`Cache-Control: no-store` under `/api/v1` by default.** So no shared cache keeps what one user was answered.
- **The desktop build's policy forbids `file:` for anything but the app's scripts, styles, and fonts.** So a `file:` address (another machine's share, on Windows) is never loaded as a picture, a video, or a request.

## Open Graph previews

See [Open Graph previews](open-graph.md).

- **The tags are rendered into the page on the server.** Unfurlers read the HTML at an address without running any script.
- **An invite page that no longer works previews as the deployment.** So a link that no longer works shows nothing of where it led.
- **The invite page shows only the community's name and icon.** Previews an unfurling service already made are its own copies, beyond the deployment's reach to take back.
- **Only PNG, JPEG, WebP, and GIF pictures are shown.** Every unfurler draws those.
- **`og:url` collapses leading slashes.** So a path such as `//other.example/x` cannot name another host.
- **A preview that cannot be read, or a page over its limits, falls back to the deployment's name rather than refusing.** Nobody is kept from the web client, and someone opening a link from a busy address still gets it.
- **The invite page has a tighter limit than other pages.** Against guessing codes.
- **The files are not limited.** They are read from disk and cost what a static file server's would.
