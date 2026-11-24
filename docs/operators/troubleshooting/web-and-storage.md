# Web client and storage

## The web client says it cannot reach the server

1. Open `https://<your domain>/api/v1/auth/methods` in a browser. It should answer JSON.
2. If it answers a page instead, your proxy sends `/api/` to something other than the API servers.

## A client is told the API answers over HTTPS only

The reverse proxy answers any request under `/api/` over plain HTTP with `403` and that text, in
English (see [Plain HTTP](../installing/4-api-server.md#plain-http)). The server never saw the
request.

1. Give the client, bot, or script the deployment's address as `public_url` has it, with `https://`.
2. Replace whatever it sent, which crossed the network unencrypted. A person signs out and in again;
   a bot's owner issues it a new token under Settings, then Developer.

## Uploads fail

The browser uploads straight to `[media.s3] public_endpoint`. It must:

- be reachable from the client,
- be served over HTTPS when the page is,
- allow the page's origin by CORS for `PUT`.

The browser's developer tools show the refused request.

## Pictures do not load

Pictures load from `[media.s3] public_base_url`. It must serve the bucket's objects without
credentials, and only them. [The storage's read path](../installing/storage-read-path.md) gives the
checks, which also show whether it lists or takes writes, as it must not.

Pictures show "Attachment unavailable" when `public_base_url` is plain `http`: clients load over
`https` alone. Serve it over `https`.

## Saving or copying a picture says the storage does not let the app read files

The apps read the file from `[media.s3] public_base_url` themselves to save or copy it, which the
browser allows only when the read path sends `Access-Control-Allow-Origin`. Add it as
[The storage's read path](../installing/storage-read-path.md#its-origin-and-headers) says, and check
it with the `curl` there.

## Videos show as downloads, without a poster

No server that makes previews can run `ffmpeg` and `ffprobe`. Each says so in its log as it starts
("makes previews of pictures only").

- Install them, or name them in `[media.previews]` (see
  [`[media.previews]`](../configuration/previews.md#mediapreviews)).
- An HDR video, or one larger than `max_video_bytes`, has no poster by design.

## SVGs show as text, not as pictures

No server that makes previews can draw SVGs. Each says why in its log as it starts ("the SVG
renderer cannot draw, so this server makes no previews of SVGs"):

- `aspen-svg-render` is not on `PATH`, or not where `[media.previews] svg_renderer` says.
- `svg_fonts` holds no fonts: build them with `scripts/svg_fonts.py`.
- Its sandbox cannot be entered: it runs only on Linux, and a container's seccomp profile must let
  it install a filter of its own.

An SVG larger than `max_svg_bytes`, or that asks for more memory or time than the renderer is given,
has no preview by design. See
[The SVG renderer and its fonts](../configuration/previews.md#the-svg-renderer-and-its-fonts).

## Messages with a picture take twenty seconds to appear

No server makes previews, so each message waits out its hold. Either:

- `[media.previews] make` or `[jobs] run` is off everywhere, or
- every server making them is stuck.

The dashboard's Jobs tab, or `aspen-chat-server jobs list`, shows `makePicturePreview` and
`makeVideoPoster` jobs waiting. See
[Jobs](server.md#something-that-should-follow-a-decision-does-not-happen).
