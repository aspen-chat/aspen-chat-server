# Previews

Readers' apps show pictures, SVGs, and videos inline from a smaller copy the server makes:

- a picture, fitted within 1920 × 960 pixels, as WebP
- an SVG, drawn at its own size, fitted the same way, as WebP
- a video's poster: one frame of it. The video itself plays only when asked.

The originals stay what the gallery shows and what is saved, but for an SVG, whose original is
served to be saved and never shown: its preview is what the gallery shows.

## How previews are made

- A message sent with a picture or video whose preview is still being made may wait for it, up to
  twenty seconds from the upload, before it is posted.
- Every server queues previews. Servers with `make` on make them, as jobs, so a server that dies
  leaves its work to the others.
- A server makes previews only with `[jobs] run` on as well (see [Jobs](jobs.md#jobs)).
- Pictures are made in the server itself.
- SVGs are drawn by running `aspen-svg-render`, with the fonts `scripts/svg_fonts.py` builds. The
  images Aspen publishes and Ultra Lite hold both. A server without them, or whose renderer fails
  the check it runs when the server starts, makes no previews of SVGs and says why; apps then show
  SVGs as text. The renderer runs only on Linux.
- Videos' posters are taken by running `ffmpeg` and `ffprobe`. They need not be installed: a server
  without them makes previews of pictures only, and says so when it starts. The images Aspen
  publishes hold neither, and set both to empty (see
  [The images](../../../deploy/README.md#the-images)).
- Each video is copied whole to a scratch file under `TMPDIR`. Point `TMPDIR` at a disk with room
  for `max_video_bytes` times `concurrency`, not a RAM-backed `/tmp`.

## `[media.previews]`

| Setting              | Default                        |                                                                                                                                                                                        |
| -------------------- | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `make`               | `true`                         | Whether this server makes previews; it needs `[jobs] run` on as well. Leave both on somewhere: with no server making them, messages sent with pictures wait their full twenty seconds. |
| `concurrency`        | `2`                            | How many previews this server makes at once. A large picture may take several hundred megabytes while it is made.                                                                      |
| `max_picture_bytes`  | `67108864` (64 MiB)            | The largest picture a preview is made of.                                                                                                                                              |
| `max_picture_pixels` | `100000000`                    | The most pixels a picture may have for a preview to be made of it.                                                                                                                     |
| `ffmpeg`, `ffprobe`  | `"ffmpeg"`, `"ffprobe"`        | The programs that take videos' posters, by path or by name on `PATH`. An empty `ffmpeg` takes no posters at all.                                                                       |
| `max_video_bytes`    | `2147483648` (2 GiB)           | The largest video a poster is taken of.                                                                                                                                                |
| `ffmpeg_memory_mib`  | `3072` (3 GiB)                 | The most memory `ffmpeg` or `ffprobe` may map while taking one poster. A frame of 8K video needs about 2 GiB.                                                                          |
| `svg_renderer`       | `"aspen-svg-render"`           | The program that draws SVGs, by path or by name on `PATH`. Empty draws no SVGs.                                                                                                        |
| `svg_fonts`          | `"/usr/share/aspen/svg-fonts"` | The directory of fonts SVGs' text is drawn in, which `scripts/svg_fonts.py` builds (below).                                                                                            |
| `max_svg_bytes`      | `8388608` (8 MiB)              | The largest SVG drawn, and the most a compressed SVG (SVGZ) may unpack to.                                                                                                             |
| `svg_memory_mib`     | `1024` (1 GiB)                 | The most memory the renderer may map while drawing one SVG, its fonts (about 60 MiB) among it.                                                                                         |

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

## The SVG renderer and its fonts

`aspen-svg-render` builds with the servers:

```
cargo build --release -p aspen-svg-render
```

Its fonts are the web client's own, written out as files it reads. With the client's dependencies
installed (`pnpm install` in `client/`) and Python's `fonttools` and `brotli`
(`pip install -r client/scripts/noto_emoji.requirements.txt`):

```
scripts/svg_fonts.py --out /usr/share/aspen/svg-fonts
```

What the server limits the renderer to, as it does `ffmpeg`:

- none of the server's environment
- `svg_memory_mib` of memory, ten seconds of CPU time, twenty seconds in all
- writing no file, and few open
- once its fonts are open, a seccomp filter of its own: reading the SVG, writing its pixels, and
  nothing else. It opens no file and no connection, whatever the SVG asks for.
- an SVG of at most `max_svg_bytes`, a million XML nodes, and embedded pictures of at most
  `max_picture_pixels`

A container's own seccomp profile must let it install its filter. Docker's and Podman's defaults do.

## Metrics

| Metric                                      | What it counts        |
| ------------------------------------------- | --------------------- |
| `aspen_attachment_previews_made_total`      | Previews made.        |
| `aspen_attachment_previews_failed_total`    | Previews that failed. |
| `aspen_attachment_preview_duration_seconds` | How long each took.   |

Each is labelled by `kind` (`picture`, `svg`, `video`).
