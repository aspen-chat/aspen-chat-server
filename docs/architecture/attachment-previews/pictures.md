# Pictures

Pictures are made in the server's own process (`aspen_previews::picture::preview`), on a blocking thread.

## Steps

1. Refuse the picture before any decoding when:
   - its header says it holds more than `[media.previews] max_picture_pixels` pixels; or
   - its original is larger than `max_picture_bytes`.
2. Check for movement. A picture that moves keeps its original (below).
3. Decode with the `image` crate's decoders, written in Rust (JPEG, PNG, WebP, GIF, TIFF, BMP), with an allocation limit.
4. Turn it upright by its EXIF orientation.
5. Bring it into sRGB from the colour profile it carries (`moxcms`), so a phone's Display P3 photo keeps its colours. A picture without a profile is taken as sRGB.
6. Resize with Lanczos (`fast_image_resize`).
7. Encode with libwebp (`webp`); see [What a preview is](preview-format.md).

**The pixel check must come before the movement check.** Telling whether a GIF moves decodes its first two frames, each the size of its whole logical screen. A GIF of a few bytes may declare that screen as 65535 by 65535.

## Pictures that keep their original

| Picture | Reason |
| --- | --- |
| One that moves (an animated GIF, APNG, or WebP) | A preview would hold its first frame only. |
| One whose colours cannot be brought into sRGB faithfully (a CMYK print file, a profile that cannot be read) | No faithful conversion to sRGB. |
| Anything the decoders do not read, such as SVG or HEIC | No decoder. |

[Design notes](design-notes.md#pictures)
