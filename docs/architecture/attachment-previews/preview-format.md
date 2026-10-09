# What a preview is

## Size

- A preview is fitted within `aspen_previews::BOX`, 1920 × 960 pixels (`aspen_previews::fit`).
- It is never enlarged, and its aspect ratio is kept.
- Apps show it inline at no more than 320 CSS pixels tall.

[Why these numbers](design-notes.md#the-preview-box)

## Encoding

- Lossy WebP at quality 85 (`aspen_previews::picture::QUALITY`).
- libwebp's sharp YUV conversion, so fine detail in saturated colour (red text on white) is not smeared.
- A preview with transparency keeps it. One whose alpha channel covers all of itself, as many screenshots' does, drops it.

## When a preview is kept

A preview is kept only when it is at least `MIN_SAVING_PERCENT`, ten percent, smaller than its original (`preview::worth_keeping`).

- An original already small and web-ready is shown as it is.
- A poster is always far smaller than its video.

How each kind is made: [Pictures](pictures.md) and [Videos](videos.md).
