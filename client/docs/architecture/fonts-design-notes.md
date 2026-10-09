# Fonts: design notes

Rationale for [Fonts](fonts.md).

## Stacks and loading

- `fontFaces.css` is imported dynamically. It is some 600 kB of rules, and importing it with the page would hold up the first paint.
- Code takes the system's monospace faces before the Noto ones. The Noto faces are proportional.

## Han characters

- `--font-han` puts the regional face for the page's language first. One Han character is drawn differently in Japanese, Korean, and Simplified, Traditional, and Hong Kong Chinese, and a message does not say which it is in.

## Emoji

- Noto Color Emoji is bundled twice, as COLRv1 and as an `sbix` bitmap build. No one colour-font format is drawn by every engine: Chromium and Firefox draw COLRv1, and WebKit draws `sbix`.
- The emoji fonts come from Google's own releases rather than Fontsource. Fontsource's build carries only SVG glyphs, which Chromium and WebKit on an iPhone draw blank.
- Each emoji font is one whole file, not `unicode-range` slices. A sequence (a family, a skin tone, a flag) is one ligature, and WebKit draws it apart when its characters come from different slices.
- The family is named `Aspen Noto Color Emoji`, a name no system font has. A page that names it in its stack without having the face (a plugin's view on a deployment that does not serve it) skips it. The system's Noto Color Emoji, if named instead, has glyphs for the digits and the space and would draw them.
- A weekly workflow runs `noto_emoji.py update`, so the fonts never wait on someone remembering.
- `e2e/fonts.spec.ts` checks pixels, not only that the font loaded. A colour font can load and still draw nothing.
- `styles.css` overrides the emoji picker's own `!important` font list with `--font-emoji`, so an emoji looks the same picked as sent.

## The user's own fonts

- The choice is kept per install, never on the account. The font files are not on the user's other devices.
- Each family is registered under an alias (`AspenFont<n>`). A system font of the same name is then never picked instead, and bold and italic come from the family's own files.
- WOFF2 is refused. It is one Brotli stream, which browsers do not offer to decompress.
- Font collections are refused. They hold several faces a `FontFace` cannot choose between.
- The library keeps file data apart from the face descriptions, so listing the library reads no font data.
- A view's page has an origin of its own, so `vite dev` sends font files for views with `Access-Control-Allow-Origin`.
