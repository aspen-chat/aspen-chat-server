# SVGs

An SVG's preview is the SVG drawn as a picture (`app::attachment::preview::svg`,
`aspen_previews::svg`). Its original is stored and served as `application/octet-stream`, to be saved
(see [Stored type](uploads.md#stored-type)), so the preview is all apps show of it, inline and in
the gallery.

## Drawing

1. Read the original from storage, up to `[media.previews] max_svg_bytes`.
2. Run `aspen-svg-render` (`[media.previews] svg_renderer`), a program of its own
   (`server/svg_render`), with the SVG on its standard input, under the limits below.
3. It maps its fonts, enters its sandbox, and only then reads the SVG.
4. It parses the SVG with `usvg` and draws it with `resvg`, at the SVG's own size (its `width` and
   `height`, or its `viewBox`), rounded to whole pixels and fitted within the preview box with its
   aspect ratio kept (`aspen_svg_render::fit`, which every preview is fitted by). A small SVG is
   drawn at its own small size, never enlarged.
5. It writes the pixels to its standard output: a header of the width and height, then 8-bit RGBA
   (`aspen_svg_render::protocol`).
6. The server reads at most the box's worth of pixels and encodes them as a picture's preview is
   (`picture::from_rgba`; see [What a preview is](preview-format.md)), with no alpha channel when
   every pixel is opaque.

The preview is kept whatever its size, since nothing else of an SVG can be shown
([When a preview is kept](preview-format.md#when-a-preview-is-kept)).

## What an SVG may reach

Nothing outside itself:

- An `<image>` naming a file or an address draws nothing (`resolve_string` answers none).
- A picture embedded as a `data:` URL is drawn, unless it has more than `max_picture_pixels`, read
  from its header, which refuses the whole SVG.
- An SVG embedded as a `data:` URL is drawn under the same rules.
- XML entities resolve inside the document only; roxmltree bounds how far they expand.

## Limits

| Limit                       | Value                                                     | Where                                                    |
| --------------------------- | --------------------------------------------------------- | -------------------------------------------------------- |
| Size of the original        | `max_svg_bytes` (8 MiB), and as much once SVGZ unpacks    | `svg::make`, `render::unpack`                            |
| XML nodes                   | A million                                                 | `protocol::MAX_NODES`                                    |
| Nesting                     | usvg's 1024 levels                                        | `usvg::DEFAULT_MAX_DEPTH`                                |
| Embedded pictures           | `max_picture_pixels` each                                 | `render::embeds_too_many_pixels`                         |
| Address space               | `svg_memory_mib` (1 GiB), the mapped fonts among it       | `RLIMIT_AS`, set as it starts (`process::limit`)         |
| CPU time                    | Ten seconds, then `SIGXCPU`, and a second later killed    | `RLIMIT_CPU`, `svg::CPU_TIME`                            |
| Wall time                   | Twenty seconds, then killed                               | `svg::DEADLINE`                                          |
| Files written, core dumps   | None                                                      | `RLIMIT_FSIZE`, `RLIMIT_CORE`                            |
| Open files                  | 16                                                        | `RLIMIT_NOFILE`                                          |
| Environment                 | None but `PATH`                                           | `process::confined`                                      |
| Pixels read back            | The box's worth, the process killed past it               | `svg::preview`                                           |
| Size drawn                  | Fitted within the preview box                             | `aspen_svg_render::fit`                                  |
| System calls once sandboxed | Reading stdin, writing stdout and stderr, memory, exiting | `aspen_svg_render::sandbox`, a seccomp filter of its own |

The sandbox is a seccomp filter the renderer installs on itself (`seccompiler`), with no new
privileges, after mapping its fonts and before reading a byte of the SVG. It allows `read` on
standard input, `write` and `writev` on standard output and error, `mmap` and `mprotect` without
`PROT_EXEC`, the other calls memory, locks, the clock, random numbers, and exiting need, and kills
the process on any other: no file can be opened, no socket made, no program run, and no memory made
executable. It exists only on Linux; elsewhere the renderer refuses to run.

## What each ending means

| The renderer                                | The preview                                                                 |
| ------------------------------------------- | --------------------------------------------------------------------------- |
| Drew it (exit 0)                            | Kept                                                                        |
| Could not read it (exit 2)                  | None: not an SVG                                                            |
| Was asked for more than its limits (exit 3) | None                                                                        |
| Cannot draw anything here (exit 4)          | Tried again later, like a storage failure: its fonts or sandbox are missing |
| Was killed past its CPU time or deadline    | None                                                                        |
| Ran out of memory or crashed                | None                                                                        |
| Was killed by its sandbox (`SIGSYS`)        | None, and a warning: drawing needs a system call the filter lacks, a bug    |
| Could not be started                        | Tried again later                                                           |

## Fonts

Text is drawn in the faces the app shows messages in: Inclusive Sans for text and every generic
family but `monospace`, Intel One Mono for `monospace`, then, for what those lack, Noto Sans, Noto
Color Emoji, the Noto face of every script the app falls back to, and the five regional CJK faces,
in that order. They are files of a directory (`svg_fonts`), which `scripts/svg_fonts.py` writes from
the web client's own fonts: the families `client/packages/app/fallbackFonts.json` names, which the
app's stylesheets are written from too, as TrueType and OpenType from the Fontsource packages'
WOFF2, which the renderer cannot read. Their file names put them in that order, which is the order a
character a face lacks is looked for in the others. Each family's licence is under `licenses/`.

The renderer maps every font before it enters its sandbox, and a directory without fonts stops it
(`Fonts::open`), so no SVG is drawn with its text left out unannounced.

Where usvg falls short: in one `<text>` element mixing a right-to-left script such as Arabic with
left-to-right letters, it gives up looking for fonts once shaping with another font gives a
different number of glyphs, so the Arabic, and whatever after it the first font lacks, draws as
boxes. Each script in a `<text>` of its own, and text converted to outlines, as most exported SVGs
have it, draws whole.

## Which servers draw them

The job is `makeSvgPreview` (see [Making previews](making-previews.md)). A server's job runner
checks its renderer when it starts (`svg::available`, the renderer's `--check`: its fonts open, its
sandbox is entered, and it draws text of its own there), and only a server whose renderer passes
claims `makeSvgPreview`. One whose renderer fails says why in its log and leaves SVGs to the servers
that can draw them; with none, SVGs wait seven days and are given up, and apps show them as text.

The API image and Ultra Lite carry the renderer and its fonts (`/usr/local/bin/aspen-svg-render` and
`/usr/share/aspen/svg-fonts`; `svg-render/` in Ultra Lite's runtime).

## SVGs uploaded before

A migration queues one `queueSvgPreviews` job, which looks through every attachment in id order, a
thousand a step, and queues a bulk `makeSvgPreview` for each that is a confirmed SVG, not evidence,
with no preview.

## What apps show

The record says the original is not served for showing in place (`Attachment.inline`, false for an
SVG and every type outside `INLINE_TYPES`). Apps show such a picture by its preview alone, inline
and in the gallery, and copy the preview when asked to copy it as a picture; saving it saves the
original. Until it has a preview, an SVG shows as the text it is (the app's text attachments include
`.svg`), and any other picture whose original is not served inline (TIFF, HEIC) as a file to save.
See the client's [attachments](../../../client/docs/architecture/message-rendering/attachments.md).

[Design notes](design-notes.md#svgs)
