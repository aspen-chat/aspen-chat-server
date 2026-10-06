# Fonts

## The faces Aspen bundles

- Text is drawn in Inclusive Sans and code in Intel One Mono, through `--font-sans` and
  `--font-mono` in `src/styles.css` (Tailwind's `font-sans` and `font-mono`, and every code
  span and block in a message). Both are Fontsource's variable packages, upright and italic,
  imported by `main.tsx`, and split by script with `unicode-range` so a page fetches only the
  slices it shows.
- What they do not draw falls back to bundled Noto faces before the system's: Noto Sans (Latin,
  Cyrillic, Greek, and Devanagari, imported by `main.tsx`), then Noto Color Emoji, then a Noto
  Sans for every script in present-day use for a language of more than ten thousand living
  speakers (Noto Serif Tibetan for Tibetan, which has no Noto Sans face), then the five
  regional CJK faces. Code takes the system's monospace faces before
  the Noto ones, which are proportional. `fallbackFonts.ts`, a Vite plugin, says which
  families are bundled and which are left out and why, and writes two stylesheets from the
  Fontsource packages and the emoji fonts when Vite starts (into `src/generated/`, which is not kept in git):
  `fontFaces.css`, each family's own script slices as WOFF2, some 600 kB of rules that
  `main.tsx` imports dynamically so they never hold up the first paint, and `fontStacks.css`,
  the custom properties the stacks are built on (`--font-emoji`, `--font-scripts`,
  `--font-han`), imported with the page. Until the faces load, text in those scripts shows in
  the system's fonts (`font-display: swap`).
- One Han character is drawn differently in Japanese, Korean, and Simplified, Traditional, and
  Hong Kong Chinese, and a message does not say which it is in, so `--font-han` puts the
  regional face first for the page's language (`<html lang>`, which follows the language Aspen
  is shown in), and Simplified, Traditional, Hong Kong, Japanese, Korean otherwise.
- No one colour-font format is drawn by every engine, so Noto Color Emoji is bundled twice, from
  Google's own releases (`googlefonts/noto-emoji`) rather than Fontsource, whose build carries
  only SVG glyphs, which Chromium and WebKit on an iPhone draw blank: Google's COLRv1 build
  (about 2 MB), which Chromium and Firefox draw, and its bitmap build with the pictures moved
  into an `sbix` table (about 9.5 MB), which WebKit draws and which is the format of Apple's own
  emoji. The one `@font-face` lists both with `tech()`, so each browser fetches only the one it
  draws. Each is one whole file, not `unicode-range` slices: a sequence (a family, a skin tone,
  a flag) is one ligature, and WebKit draws it apart when its characters come from different
  slices. `scripts/noto_emoji.py` builds them into `packages/app/fonts/noto-color-emoji/`, kept
  in git with the licence and a `manifest.json` naming the upstream commit and its files'
  hashes; `build` makes the same files again from that commit, and `update` builds from the
  newest commit that changed the fonts. `.github/workflows/emoji-font.yml` runs `update` every
  week and opens a pull request when upstream has changed them, so they never wait on someone
  remembering. `e2e/fonts.spec.ts` draws emoji in the face and checks the pixels are coloured
  and a family of four takes one advance, since a colour font can load and still draw nothing.
- The emoji face's family is `Aspen Noto Color Emoji`, a name no system font has: a page that
  names it in its stack without having the face (a plugin's view on a deployment that does not
  serve it) skips it, where the system's Noto Color Emoji has glyphs for the digits and the space
  and would draw them.
- Plugins' views draw in the same faces. `src/faces.css` holds Aspen's own (which `main.tsx`
  imports) and `src/viewFonts.css` those and every fallback face; `viewFonts.ts` builds the
  second as an entry of its own, written to `view-fonts.css` at the web client's root under that
  one name, which the bridge hands each view (`plugins.md`), and answers the same name in `vite
  dev`, sending font files with `Access-Control-Allow-Origin`, since a view's page has an origin
  of its own.
- The emoji picker names system emoji fonts of its own with `!important`;
  `styles.css` overrides it with `--font-emoji`, so an emoji looks the same picked as sent.
- Every bundled font's licence is on the Open Source Attributions page with every other
  package's (`about.md`): each `@fontsource` dependency's, and the emoji fonts' directory's.
- Bundled fonts come to about 39 MB of WOFF2 in the build, nearly all of it CJK and emoji. The
  web client fetches only the slices a page draws, and of the emoji fonts the one its browser
  draws, whole, once the page shows an emoji; the desktop and mobile apps ship them all.

## The user's own fonts

- Settings, Fonts (`FontsSection`) chooses the font for text and for code on this install,
  each Aspen's own or a family from the user's font library; the select lists each family in
  itself. The choice is remembered per install in `localStorage` (`aspen.font.text`,
  `aspen.font.code`), like the palette, and never follows the account, since the files are not
  on the user's other devices. It applies before sign-in too (`applyStoredFonts` in
  `main.tsx`).
- The library (`theme/fontLibrary.ts`) keeps the files the user adds, whole, in IndexedDB
  (`aspen-fonts`: what each face says of itself in `faces`, its file apart in `files`, so
  listing the library reads no font data), and asks the browser to keep that storage. Each file is read for what it
  says about itself (`theme/fontFile.ts`): its family (the typographic family, or the legacy
  family where it has none, from the `name` table), its weight (`OS/2`, or the range of a
  variable font's `wght` axis), and whether it is italic or oblique; files group into families
  by that name, and a face of a family, weight, and style already kept is replaced. TrueType,
  OpenType, and WOFF files are read; WOFF2 (one Brotli stream, which browsers do not offer to
  decompress) and font collections (several faces a `FontFace` cannot choose between) are
  refused, as is anything unreadable, each with a reason naming the file.
- A family is registered with `document.fonts` under an alias of its own (`AspenFont<n>`) when
  first used, one `FontFace` per file with its weight and style, so bold and italic come from
  the family's own files where it has them and a system font of the same name is never picked
  instead. `applyFont` sets `--font-text-chosen` or `--font-code-chosen` on `<html>` to the
  alias, which `--font-sans` and `--font-mono` put first, ahead of the bundled faces, which
  still draw what the user's font lacks. A family removed, or none of whose files the browser
  can draw, falls back to Aspen's own and is forgotten; a library that cannot be read leaves
  the choice remembered for the next launch. A plugin's view, which cannot read the library, is
  handed the files of the families drawn now under their aliases (`facesUnder`, through the
  bridge), and registers them itself.
