# Web client and storage

## The web client says it cannot reach the server

1. Open `https://<your domain>/api/v1/auth/methods` in a browser. It should answer JSON.
2. If it answers a page instead, your proxy sends `/api/` to something other than the API
   servers.

## Uploads fail

The browser uploads straight to `[media.s3] public_endpoint`. It must:

- be reachable from the client,
- be served over HTTPS when the page is,
- allow the page's origin by CORS for `PUT`.

The browser's developer tools show the refused request.

## Pictures do not load

Pictures load from `[media.s3] public_base_url`. It must serve the bucket's objects without
credentials, and only them. [The storage's read path](../installing/storage-read-path.md) gives
the checks, which also show whether it lists or takes writes, as it must not.

## Videos show as downloads, without a poster

No server that makes previews can run `ffmpeg` and `ffprobe`. Each says so in its log as it
starts ("makes previews of pictures only").

- Install them, or name them in `[media.previews]` (see
  [`[media.previews]`](../configuration/previews.md#mediapreviews)).
- An HDR video, or one larger than `max_video_bytes`, has no poster by design.

## Messages with a picture take twenty seconds to appear

No server makes previews, so each message waits out its hold. Either:

- `[media.previews] make` or `[jobs] run` is off everywhere, or
- every server making them is stuck.

The dashboard's Jobs tab, or `aspen-chat-server jobs list`, shows `makePicturePreview` and
`makeVideoPoster` jobs waiting. See [Jobs](server.md#something-that-should-follow-a-decision-does-not-happen).
