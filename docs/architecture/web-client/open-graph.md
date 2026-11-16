# Open Graph previews

Chat apps, social networks, and search engines preview a link from the HTML at its address, without
running any script. So the server puts the preview's tags into the page.

| Part                    | Where                                                         |
| ----------------------- | ------------------------------------------------------------- |
| What a page previews as | `app::open_graph`, `app::open_graph::of_invite`               |
| The tags' template      | `server/templates/web_client/open_graph.html` (askama)        |
| The invite page's limit | `api::rate_limit::within_limits`                              |
| The Aspen mark          | `open-graph.png`, written by `client/scripts/export_icons.py` |

## The page and its tags

The page is the built `index.html` with its `<title>` taken out. Before its `</head>` the askama
template `server/templates/web_client/open_graph.html` renders:

- A title.
- A description.
- Open Graph tags: `og:site_name`, `og:title`, `og:description`, `og:url`, `og:image` with its type
  and alt text, and `twitter:card` `summary`.
- The deployment's profile, as `GET /deployment` answers it, in a `<script type="application/json">`
  whose `id` is the one `spec/web_client_page.json` names, and a `<link rel="preload" as="image">`
  of its icon when it has one. The sign-in screen shows both before its script could have read them
  (see `client/docs/architecture/sign-in.md`).

The template escapes every value. The profile is JSON with `<`, `>`, `&`, U+2028, and U+2029 written
as escapes (`api::web_client::script_json`), so no text a deployment's administrators give it can
end its element. The Content Security Policy runs no inline script; a JSON element is data, read by
the page's own script. The page is in the request's language (`Accept-Language`, as every response
is).

## What a page previews as

`app::open_graph` decides it. The deployment's name and icon come from the server's copy of the
settings (`SettingsCache`), so every page but a working invite's reads nothing.

| Page                      | Title                                                              | Picture               |
| ------------------------- | ------------------------------------------------------------------ | --------------------- |
| Every page                | The deployment's display name (or "Aspen"), as title and site name | The deployment's icon |
| An invite page that works | The invite's community's name                                      | The community's icon  |

An invite page is `/invite/{code}` without `?at=`, or with `at` naming this deployment's federation
domain. It previews as the community while:

- The invite is not revoked.
- The invite is not expired.
- The community is not deleted.

Otherwise it previews as the deployment, so a link that no longer works shows nothing of where it
led.

The read is one query joining the invite, its community, and the community's icon once its upload is
confirmed.

### Pictures

- A picture is shown only when its type is one every unfurler draws: PNG, JPEG, WebP, GIF.
- A community without one falls back to the deployment's icon.
- A deployment without one falls back to the Aspen mark, `open-graph.png` (512 pixels square).
  `client/scripts/export_icons.py` writes it into the web client's `public/`.

### `og:url`

`og:url` is the request's path and query on `public_url`'s origin, its leading slashes collapsed to
one. So a path such as `//other.example/x` cannot name another host.

### When the preview cannot be read

An invite whose preview cannot be read (the database unreachable, say) leaves the page the
deployment's, from the server's copy of the settings. It does not keep anyone from the web client.
Only a missing or unreadable `index.html` answers `500`.

## Limits

Pages are never refused. The invite page, `GET /invite/{code}`, checks its limit in its handler
through `api::rate_limit::within_limits`: 60 a minute per address, burst 30, against guessing codes.
Over it, the page previews as the deployment's, without reading the invite. Someone opening a link
from a busy address still gets the web client.

Every other page, and the files, are not limited. A page is made from `index.html` and the server's
copy of the settings, and the files are read from disk; each costs what a static file server's
would.

## When access is given or taken away

1. **Who can observe it, and by which routes?** Anyone who has an invite link, signed in or not, and
   every service that unfurls it, sees the invite's community's name and icon in the page at
   `/invite/{code}`. Nothing else of the community: not members, channels, the invite's maker, or
   its expiry. Everyone sees the deployment's name and icon, which `GET /deployment` already gives
   anyone.
2. **What decides it, and where is that checked?** Holding a working code, checked in the query of
   `app::open_graph::of_invite` (not revoked, not expired, community not deleted) on every request.
3. **When it is lost, what happens to what is already open?** Nothing is held open. A revoked or
   expired invite, a deleted community, or a changed name or icon shows in the next request for the
   page, since the invite is read on every request, the deployment's profile is the server's copy,
   which follows every change as it commits, and the page is `no-cache`. Previews an unfurling
   service already made are its own copies and stay as they were until it fetches again. That is
   outside the deployment's reach, and is why the page shows nothing more than the community's name
   and icon.
4. **When it is gained, how does a client already open find out?** A new invite or a renamed
   community previews as such on the next fetch.
5. **Does every path that changes it announce it?** Nothing is announced. The page reads the invite
   from the database each time; every change to the deployment's profile, from the dashboard, the
   terminal, or the `shrinkIcons` job, rings the settings bucket that each server's copy follows.
6. **Is it published inside the transaction that makes the change?** Nothing is published.

`scripts/check_permissions.py` checks that an invite's page shows its community, a rename at once,
and nothing of the community once the invite is revoked or expired or the community deleted, and
(`deployment_profile`) that every page carries the profile `GET /deployment` gives, following a
change from the dashboard, the terminal, and the `shrinkIcons` job.
