# Step 5: Serving the web client

Each API server serves the web client itself, at `public_url` beside the API:

- its files, from `[web_client] dir` (see [`[web_client]`](../configuration/address.md#web_client));
- every other path the API does not own, with its page. The web client's links are real paths,
  such as `/communities/…`.

## Reverse proxy

A reverse proxy in front sends everything to the API servers, including the WebSocket at
`/api/v1/events`. Pass `Upgrade` and `Connection` through. With Caddy:

```
chat.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

## Caching

| Files | Caching |
| --- | --- |
| Under `/assets/` | Named by their contents, and sent to be cached for good. |
| Everything else | Revalidated on each load. |

## Security headers

The server sends the web client with a Content Security Policy and these headers:

- `nosniff`,
- no referrer,
- framing refused,
- no window shared with another page,
- HSTS for a year, when `public_url` is `https`. **Once a browser has seen it, it reaches your
  deployment over HTTPS only.**

Every API answer carries `nosniff` and, except the few meant to be kept,
`Cache-Control: no-store`.

The policy is built from your configuration, so nothing needs adding by hand. It allows:

- your storage's `public_base_url` (in `[media.s3]`) for pictures and videos,
- its `public_endpoint` (or `endpoint`) for uploads.

**Let your reverse proxy pass these headers through, rather than setting its own.** A second
policy is applied as well as the first, and one that leaves out your storage breaks pictures and
uploads.

## Link previews

Each page is sent with link preview tags (Open Graph), for chat apps, social networks, and search
engines that preview a link without running the web client.

| Link | Preview shows |
| --- | --- |
| Any page | Your deployment's name and icon. Without an icon, the Aspen mark (`open-graph.png` in the web client). |
| A working invite link | Its community's name and icon, to anyone who has the link, signed in or not. |
| A revoked or expired invite | The deployment. |

Services that unfurl links keep their own copy of a preview for a while. A renamed or deleted
community can still show in previews made before.

## Releasing a new web client

The server reads the web client's files as they are asked for, so a new release needs no
restart.

1. Copy the new build over the old on each API server, `index.html` last. Copy over the old
   directory rather than replacing it: browsers that already have the web client open load some
   parts later (the code highlighting for each language, the QR code reader), from the release
   they started with.
2. Once a release has been out for a few days, remove old files from `assets/`: delete what the
   newest build does not have.

---

Previous: [Step 4: Starting the API server](4-api-server.md) · Next:
[Step 6: Voice servers](6-voice-servers.md)
