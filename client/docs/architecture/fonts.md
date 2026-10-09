# Fonts

Aspen bundles its own text and code faces, with Noto fallbacks for every present-day script and colour emoji. A user may also choose fonts from their own library, per install.

Design notes: [fonts-design-notes.md](fonts-design-notes.md).

## Where it lives

| Part | Code |
| --- | --- |
| Font stacks | `--font-sans`, `--font-mono` in `src/styles.css` |
| Bundled-family list and generated stylesheets | `fallbackFonts.ts` (a Vite plugin), writing into `src/generated/` |
| Aspen's own faces | `src/faces.css` |
| Faces for plugins' views | `src/viewFonts.css`, built by `viewFonts.ts` |
| Emoji font builder | `scripts/noto_emoji.py`, output in `packages/app/fonts/noto-color-emoji/` |
| Emoji font updates | `.github/workflows/emoji-font.yml` |
| Settings | `FontsSection` |
| The user's font library | `theme/fontLibrary.ts`, `theme/fontFile.ts` |
| Tests | `e2e/fonts.spec.ts` |

## The faces Aspen bundles

### Text and code

| Use | Face | Property | Tailwind |
| --- | --- | --- | --- |
| Text | Inclusive Sans | `--font-sans` | `font-sans` |
| Code (and every code span and block in a message) | Intel One Mono | `--font-mono` | `font-mono` |

- Both are Fontsource's variable packages, upright and italic, imported by `main.tsx`.
- Both are split by script with `unicode-range`, so a page fetches only the slices it shows.

### Fallback order

What the text face does not draw falls back to bundled Noto faces before the system's, in this order:

1. Noto Sans: Latin, Cyrillic, Greek, and Devanagari, imported by `main.tsx`.
2. Noto Color Emoji.
3. A Noto Sans for every script in present-day use for a language of more than ten thousand living speakers. Tibetan uses Noto Serif Tibetan, since it has no Noto Sans face.
4. The five regional CJK faces.

Code takes the system's monospace faces before the Noto ones, since the Noto ones are proportional.

### Generated stylesheets

`fallbackFonts.ts` says which families are bundled and which are left out, and why. When Vite starts, it writes two stylesheets from the Fontsource packages and the emoji fonts into `src/generated/` (not kept in git):

| File | Contents | Loaded |
| --- | --- | --- |
| `fontFaces.css` | Each family's own script slices as WOFF2; some 600 kB of rules | Imported dynamically by `main.tsx`, so it never holds up the first paint |
| `fontStacks.css` | The custom properties the stacks are built on: `--font-emoji`, `--font-scripts`, `--font-han` | Imported with the page |

Until the faces load, text in those scripts shows in the system's fonts (`font-display: swap`).

### Han characters

`--font-han` puts the regional face first for the page's language (`<html lang>`, which follows the language Aspen is shown in). Otherwise the order is Simplified Chinese, Traditional Chinese, Hong Kong Chinese, Japanese, Korean.

**Why:** one Han character is drawn differently in Japanese, Korean, and Simplified, Traditional, and Hong Kong Chinese, and a message does not say which it is in.

### Emoji

Noto Color Emoji is bundled twice, from Google's own releases (`googlefonts/noto-emoji`):

| Build | Size | Drawn by |
| --- | --- | --- |
| COLRv1 | About 2 MB | Chromium and Firefox |
| Bitmap, pictures moved into an `sbix` table (the format of Apple's own emoji) | About 9.5 MB | WebKit |

- The one `@font-face` lists both with `tech()`, so each browser fetches only the one it draws.
- Each is one whole file, not `unicode-range` slices.
- The family is `Aspen Noto Color Emoji`, a name no system font has.
- The emoji picker names system emoji fonts of its own with `!important`. `styles.css` overrides it with `--font-emoji`, so an emoji looks the same picked as sent.

See the [design notes](fonts-design-notes.md#emoji) for why each of these holds.

#### Building and updating the emoji fonts

`scripts/noto_emoji.py` builds them into `packages/app/fonts/noto-color-emoji/`. That directory is kept in git, with the licence and a `manifest.json` naming the upstream commit and its files' hashes.

| Command | What it does |
| --- | --- |
| `build` | Makes the same files again from the manifest's commit |
| `update` | Builds from the newest upstream commit that changed the fonts |

`.github/workflows/emoji-font.yml` runs `update` every week and opens a pull request when upstream has changed them.

#### Testing

`e2e/fonts.spec.ts` draws emoji in the face and checks:

- the pixels are coloured
- a family of four takes one advance

**Why:** a colour font can load and still draw nothing.

### Plugins' views

Plugins' views draw in the same faces.

- `src/faces.css` holds Aspen's own faces, and `main.tsx` imports it.
- `src/viewFonts.css` holds those and every fallback face.
- `viewFonts.ts` builds `viewFonts.css` as an entry of its own, written to `view-fonts.css` at the web client's root under that one name. The bridge hands that name to each view (see [Plugins](plugins/channels-and-views.md#the-theme)).
- In `vite dev`, `viewFonts.ts` answers the same name, sending font files with `Access-Control-Allow-Origin`, since a view's page has an origin of its own.

### Licences

Every bundled font's licence is on the Open Source Attributions page with every other package's (see [About Aspen](about.md)): each `@fontsource` dependency's, and the emoji fonts' directory's.

### Size

Bundled fonts come to about 39 MB of WOFF2 in the build, nearly all of it CJK and emoji.

| Shell | What it fetches |
| --- | --- |
| Web client | Only the slices a page draws. Of the emoji fonts, only the one its browser draws, whole, once the page shows an emoji. |
| Desktop and mobile apps | All of them, shipped |

## The user's own fonts

### Choosing

Settings, Fonts (`FontsSection`) chooses the font for text and for code on this install. Each is Aspen's own or a family from the user's font library. The select lists each family in itself.

- The choice is remembered per install in `localStorage` (`aspen.font.text`, `aspen.font.code`), like the palette.
- It never follows the account, since the files are not on the user's other devices.
- It applies before sign-in too (`applyStoredFonts` in `main.tsx`).

### The library

`theme/fontLibrary.ts` keeps the files the user adds, whole, in IndexedDB database `aspen-fonts`:

| Store | Holds |
| --- | --- |
| `faces` | What each face says of itself |
| `files` | Each face's file, apart, so listing the library reads no font data |

It asks the browser to keep that storage.

### Reading a file

`theme/fontFile.ts` reads what each file says about itself:

- its family: the typographic family, or the legacy family where it has none, from the `name` table
- its weight: from `OS/2`, or the range of a variable font's `wght` axis
- whether it is italic or oblique

Files group into families by that name. A face of a family, weight, and style already kept is replaced.

| Format | Accepted |
| --- | --- |
| TrueType, OpenType, WOFF | Yes |
| WOFF2 | No: one Brotli stream, which browsers do not offer to decompress |
| Font collections | No: several faces a `FontFace` cannot choose between |
| Anything unreadable | No |

Each refusal gives a reason naming the file.

### Applying a family

1. When first used, the family is registered with `document.fonts` under an alias of its own (`AspenFont<n>`). There is one `FontFace` per file, with its weight and style, so bold and italic come from the family's own files where it has them, and a system font of the same name is never picked instead.
2. `applyFont` sets `--font-text-chosen` or `--font-code-chosen` on `<html>` to the alias.
3. `--font-sans` and `--font-mono` put the chosen alias first, ahead of the bundled faces, which still draw what the user's font lacks.

### When a family cannot be used

- A family removed, or none of whose files the browser can draw, falls back to Aspen's own and is forgotten.
- A library that cannot be read leaves the choice remembered for the next launch.

### In plugins' views

A plugin's view cannot read the library. It is handed the files of the families drawn now, under their aliases (`facesUnder`, through the bridge), and registers them itself.
