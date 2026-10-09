# Previews

Readers' apps show pictures and videos inline from a smaller copy the server makes:

- a picture, fitted within 1920 × 960 pixels, as WebP
- a video's poster: one frame of it. The video itself plays only when asked.

The originals stay what the gallery shows and what is saved.

## How previews are made

- A message sent with a picture or video whose preview is still being made may wait for it, up
  to twenty seconds from the upload, before it is posted.
- Every server queues previews. Servers with `make` on make them, as jobs, so a server that dies
  leaves its work to the others.
- A server makes previews only with `[jobs] run` on as well (see [Jobs](jobs.md#jobs)).
- Pictures are made in the server itself.
- Videos' posters are taken by running `ffmpeg` and `ffprobe`. They need not be installed: a
  server without them makes previews of pictures only, and says so when it starts.
- Each video is copied whole to a scratch file under `TMPDIR`. Point `TMPDIR` at a disk with room
  for `max_video_bytes` times `concurrency`, not a RAM-backed `/tmp`.

## `[media.previews]`

| Setting | Default | |
| --- | --- | --- |
| `make` | `true` | Whether this server makes previews; it needs `[jobs] run` on as well. Leave both on somewhere: with no server making them, messages sent with pictures wait their full twenty seconds. |
| `concurrency` | `2` | How many previews this server makes at once. A large picture may take several hundred megabytes while it is made. |
| `max_picture_bytes` | `67108864` (64 MiB) | The largest picture a preview is made of. |
| `max_picture_pixels` | `100000000` | The most pixels a picture may have for a preview to be made of it. |
| `ffmpeg`, `ffprobe` | `"ffmpeg"`, `"ffprobe"` | The programs that take videos' posters, by path or by name on `PATH`. An empty `ffmpeg` takes no posters at all. |
| `max_video_bytes` | `2147483648` (2 GiB) | The largest video a poster is taken of. |
| `ffmpeg_memory_mib` | `3072` (3 GiB) | The most memory `ffmpeg` or `ffprobe` may map while taking one poster. A frame of 8K video needs about 2 GiB. |

## Running `ffmpeg` safely

What the server already limits `ffmpeg` and `ffprobe` to:

- none of the server's environment
- reading only the video they are given
- decoding only the codecs phones, cameras, and screen recorders write
- frames of at most `max_picture_pixels`; a frame larger than `max_picture_bytes` is not read
- one thread, for at most a minute of CPU time
- writing no file

What you should do as well:

- **Run the server as a user that cannot read anything it does not need**: not `aspen.toml`'s
  secrets beyond what the server reads at startup, nor other services' files.
- Or run it in a container or sandbox of its own.
- Or leave `ffmpeg` empty to take no posters at all.

## Metrics

| Metric | What it counts |
| --- | --- |
| `aspen_attachment_previews_made_total` | Previews made. |
| `aspen_attachment_previews_failed_total` | Previews that failed. |
| `aspen_attachment_preview_duration_seconds` | How long each took. |

Each is labelled by `kind` (`picture`, `video`).
