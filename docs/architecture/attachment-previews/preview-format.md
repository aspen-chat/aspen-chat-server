# What a preview is

## Size

- A preview is fitted within `aspen_previews::BOX`, 1920 × 960 pixels (`aspen_previews::fit`, which
  is `aspen_svg_render::fit`, so the SVG renderer fits drawings the same way).
- It is never enlarged, and its aspect ratio is kept.
- Apps show it inline at no more than 320 CSS pixels tall.

[Why these numbers](design-notes.md#the-preview-box)

## Encoding

- Lossy WebP at quality 85 (`aspen_previews::picture::QUALITY`).
- libwebp's sharp YUV conversion, so fine detail in saturated colour (red text on white) is not
  smeared.
- A preview with transparency keeps it. One whose alpha channel covers all of itself, as many
  screenshots' does, drops it.

## When a preview is kept

A preview of an original served for showing in place is kept only when it is at least
`MIN_SAVING_PERCENT`, ten percent, smaller than it (`preview::worth_keeping`,
`preview::smaller_enough`). One of an original served to be saved, an SVG's among them, is kept
whatever its size, since apps can show nothing else of it.

- An original already small and web-ready is shown as it is.
- A poster is always far smaller than its video.
- An SVG's preview is often larger than the SVG.

How each kind is made: [Pictures](pictures.md), [SVGs](svgs.md), and [Videos](videos.md).
