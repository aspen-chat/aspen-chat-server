# The web client

A deployment is one origin, `public_url`: every API server serves the web client beside the API (`api::web_client`), so links, passkeys, and the pages the apps open all name one address, and the web client calls the API at the origin it was loaded from. `[web_client] dir` is the built web client (`pnpm build`), and the server refuses to start unless it holds an `index.html` with a `</head>` and `open-graph.png` (`web_client::check`), except as a private worker (`--private-worker`, `app::context::Role::PrivateWorker`), which opens no listening socket, leaves its event feed idle, and does only the background work every API server shares, so it serves no web client. The development stack keeps the same shape with the Vite dev server in front: its `public_url` is Vite's origin, which proxies the server's own paths to it.

## Serving

The routes the server owns are the API under `/api/v1` (and every other path under `/api/`, a `404` Problem rather than a page, so a newer client's request is not answered with HTML), `/auth/passkey`, `/.well-known/aspen`, `/email/unsubscribe`, and `/invite/{code}`, the invite page. Every other path falls to `web_client::files`: a `tower_http` `ServeDir` over the directory, which serves a file where there is one, and the page where there is none, directories included, so `/` is the page. A missing file under `/assets/` is a `404` rather than the page, since a page of an older release asking for its own script cannot run HTML. Files under `/assets/`, which the build names by their contents, are sent `Cache-Control: public, max-age=31536000, immutable`; everything else, the page included, `no-cache`, so a new release reaches browsers on their next load. Everything is read from the directory as it is asked for, so a release copied into place is served at once, with no restart. CORS allows every origin (`api::cors_layer`): the web client needs none, but the desktop app's pages (`file:`), the mobile apps', and other deployments' web clients call the API from elsewhere, and every request is authenticated by a bearer token rather than a cookie, so no origin gains anything a page could not already do with the token it holds.

## The page and its tags

Chat apps, social networks, and search engines preview a link from the HTML at its address, without running any script, so the page is the built `index.html` with its `<title>` taken out and, before its `</head>`, a title, a description, and Open Graph tags (`og:site_name`, `og:title`, `og:description`, `og:url`, `og:image` with its type and alt text, and `twitter:card` `summary`) rendered by the askama template `server/templates/web_client/open_graph.html`, which escapes every value. It is in the request's language (`Accept-Language`, as every response is).

## What a page previews as

`app::open_graph` decides it. Every page previews as the deployment: its display name (or "Aspen") as title and site name, and its icon. An invite page, `/invite/{code}` without `?at=` or with `at` naming this deployment's federation domain, previews as the invite's community, its name as title and its icon, while the invite is neither revoked nor expired and the community not deleted; otherwise as the deployment, so a link that no longer works shows nothing of where it led. The read is one query joining the invite, its community, and the community's icon once its upload is confirmed. A picture is shown only when its type is one every unfurler draws (PNG, JPEG, WebP, GIF); a community without one falls back to the deployment's icon, and a deployment without one to the Aspen mark, `open-graph.png` (512 pixels square), which `client/scripts/export_icons.py` writes into the web client's `public/`. `og:url` is the request's path and query on `public_url`.

A preview that cannot be read (the database unreachable, say) leaves the page the deployment's by name alone, from the server's copy of the settings, rather than keeping anyone from the web client; only a missing or unreadable `index.html` answers `500`.

## Limits

Pages are never refused: each checks its limits in its handler through `api::rate_limit::within_limits` and over them is served without the read it would make. A deployment's page (`GET /{*path}`, the default per-address budget) then previews by the deployment's name alone, from the server's copy of the settings, without reading its icon; the invite page (`GET /invite/{code}`, 60 a minute per address, burst 30, against guessing codes) previews as the deployment's without reading the invite. Someone opening a link from a busy address still gets the web client. The files are not limited: they are read from disk and cost what a static file server's would.

## When access is given or taken away

1. **Who can observe it, and by which routes?** Anyone who has an invite link, signed in or not, and every service that unfurls it, sees the invite's community's name and icon in the page at `/invite/{code}`; nothing else of the community (members, channels, the invite's maker or expiry). Everyone sees the deployment's name and icon, which `GET /deployment` already gives anyone.
2. **What decides it, and where is that checked?** Holding a working code, checked in the query of `app::open_graph::of_invite` (not revoked, not expired, community not deleted) on every request.
3. **When it is lost, what happens to what is already open?** Nothing is held open: a revoked or expired invite, a deleted community, or a changed name or icon shows in the next request for the page, since nothing is cached on the server and the page is `no-cache`. Previews an unfurling service already made are its own copies and stay as they were until it fetches again; that is outside the deployment's reach, and is why the page shows nothing more than the community's name and icon.
4. **When it is gained, how does a client already open find out?** A new invite or a renamed community previews as such on the next fetch.
5. **Does every path that changes it announce it?** Nothing is announced; the page reads the database each time.
6. **Is it published inside the transaction that makes the change?** Nothing is published.

`scripts/check_permissions.py` checks that an invite's page shows its community, a rename at once, and nothing of the community once the invite is revoked or expired or the community deleted.
