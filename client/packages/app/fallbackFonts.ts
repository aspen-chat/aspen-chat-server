import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";

/**
 * The faces Aspen falls back to for whatever its own typefaces do not draw: a Noto Sans for
 * every script in present-day use for a language of more than ten thousand living speakers
 * (Noto Serif Tibetan for Tibetan, which has no Noto Sans face) and the five regional Noto Sans
 * CJK faces, bundled from Fontsource, and Noto Color Emoji, built from Google's own releases by
 * `scripts/noto_emoji.py` into `fonts/noto-color-emoji/` (Fontsource's build of it carries only
 * SVG glyphs, which Chromium and WebKit on an iPhone draw blank).
 *
 * Fontsource's own stylesheets would bring each family's Latin, Cyrillic, maths, and symbol
 * slices too, which Inclusive Sans and Noto Sans already draw, and its static packages name a
 * WOFF beside every WOFF2. So this plugin writes two stylesheets from them when Vite starts:
 * `src/generated/fontFaces.css`, each family's own script slices (named, or numbered where a
 * large family is cut by Unicode range) in WOFF2 alone and the emoji face, some 600 kB of rules
 * that `main.tsx`
 * loads without holding up the first paint; and `src/generated/fontStacks.css`, the custom
 * properties `styles.css` builds its font stacks on, loaded with the page: `--font-emoji`,
 * `--font-scripts` (in the order below), and `--font-han`, whose regional face comes first for
 * the page's language (`<html lang>`), since one Han character is drawn differently in
 * Japanese, Korean, and Simplified, Traditional, and Hong Kong Chinese, and a message does not
 * say which it is in.
 *
 * Left out: scripts now written only for history or liturgy (Kaithi, Modi, Sharada, Takri,
 * Grantha, Rejang, Osmanya, and the like), those of dead languages, those whose languages have
 * fewer speakers (Cherokee, Osage, Buhid, Tagbanwa), Devanagari, which Noto Sans has, and
 * stylistic variants (looped Thai and Lao, unjoined Adlam).
 */

/** Variable families, by Fontsource id after `noto-sans-`. */
const VARIABLE = [
  "adlam",
  "arabic",
  "armenian",
  "balinese",
  "bamum",
  "bassa-vah",
  "bengali",
  "canadian-aboriginal",
  "cham",
  "ethiopic",
  "georgian",
  "gujarati",
  "gunjala-gondi",
  "gurmukhi",
  "hanifi-rohingya",
  "hebrew",
  "javanese",
  "kannada",
  "kayah-li",
  "khmer",
  "lao",
  "lisu",
  "malayalam",
  "meetei-mayek",
  "myanmar",
  "nag-mundari",
  "new-tai-lue",
  "ol-chiki",
  "oriya",
  "sinhala",
  "sora-sompeng",
  "sundanese",
  "syriac",
  "tai-tham",
  "tamil",
  "tangsa",
  "telugu",
  "thaana",
  "thai",
];

/** Families Fontsource has in one weight only, by id after `noto-sans-`. */
const STATIC = [
  "batak",
  "buginese",
  "chakma",
  "hanunoo",
  "lepcha",
  "limbu",
  "masaram-gondi",
  "mende-kikakui",
  "miao",
  "mongolian",
  "mro",
  "newa",
  "nko",
  "pahawh-hmong",
  "pau-cin-hau",
  "saurashtra",
  "sunuwar",
  "syloti-nagri",
  "tagalog",
  "tai-le",
  "tai-viet",
  "tifinagh",
  "tirhuta",
  "vai",
  "wancho",
  "warang-citi",
  "yi",
];

/** The regional CJK faces, by id after `noto-sans-`, in the order a page of no CJK language uses. */
const HAN = ["sc", "tc", "hk", "jp", "kr"] as const;
type Han = (typeof HAN)[number];

/** The languages whose readers expect one regional face first, as `:lang()` arguments. */
const HAN_LANGUAGES: Record<Han, readonly string[]> = {
  sc: ["zh-Hans", "zh-CN", "zh-SG"],
  tc: ["zh-Hant", "zh-TW"],
  // After Traditional, so a Hong Kong page that names its script too still finds its own.
  hk: ["zh-HK", "zh-MO", "zh-Hant-HK", "zh-Hant-MO"],
  jp: ["ja"],
  kr: ["ko"],
};

/** Slices of a family that the faces before it already draw, or that are not its script. */
const SHARED_SUBSETS = new Set([
  "latin",
  "latin-ext",
  "cyrillic",
  "cyrillic-ext",
  "greek",
  "greek-ext",
  "vietnamese",
  "math",
  "symbols",
]);

interface Family {
  /** The package, as imported. */
  name: string;
  /** Its id, which begins each slice's comment in its stylesheet. */
  id: string;
}

const scripts: Family[] = [
  ...VARIABLE.map((id) => ({
    name: `@fontsource-variable/noto-sans-${id}`,
    id: `noto-sans-${id}`,
  })),
  ...STATIC.map((id) => ({ name: `@fontsource/noto-sans-${id}`, id: `noto-sans-${id}` })),
  { name: "@fontsource-variable/noto-serif-tibetan", id: "noto-serif-tibetan" },
];
const han: Record<Han, Family> = Object.fromEntries(
  HAN.map((id) => [id, { name: `@fontsource-variable/noto-sans-${id}`, id: `noto-sans-${id}` }]),
) as Record<Han, Family>;
const here = dirname(fileURLToPath(import.meta.url));
const generated = join(here, "src/generated");
const emojiDirectory = join(here, "fonts/noto-color-emoji");
const require = createRequire(import.meta.url);

/** Each `@font-face` in a Fontsource stylesheet, after the comment naming its slice. */
const FACE = /\/\* (\S+) \*\/\s*@font-face \{([^}]*)\}/g;
const FAMILY = /font-family: '([^']+)'/;
const WOFF2 = /url\(\.\/(files\/[^)]+\.woff2)\) format\('(woff2(?:-variations)?)'\)/;
const SRC = /src: [^;]+;/;

/** The family's own slices from its stylesheet, as `@font-face` rules naming WOFF2 alone. */
function faces(family: Family): { css: string; fontFamily: string } {
  const stylesheet = require.resolve(family.name);
  const directory = relative(generated, dirname(stylesheet)).split("\\").join("/");
  let fontFamily: string | undefined;
  const rules: string[] = [];
  for (const [, slice = "", body = ""] of readFileSync(stylesheet, "utf8").matchAll(FACE)) {
    // `noto-sans-arabic-arabic-wght-normal`, `noto-sans-jp-[12]-wght-normal`.
    const subset = slice.slice(family.id.length + 1).replace(/-(wght|\d+)-(normal|italic)$/, "");
    if (SHARED_SUBSETS.has(subset)) {
      continue;
    }
    const woff2 = WOFF2.exec(body);
    const name = FAMILY.exec(body)?.[1];
    if (woff2 === null || name === undefined) {
      throw new Error(`${family.name}: unexpected @font-face for ${slice}`);
    }
    fontFamily ??= name;
    const src = `src: url("${directory}/${woff2[1] ?? ""}") format("${woff2[2] ?? ""}");`;
    rules.push(`@font-face {${body.replace(SRC, src)}}`);
  }
  if (fontFamily === undefined) {
    throw new Error(`${family.name}: no slices of its own`);
  }
  return { css: rules.join("\n"), fontFamily };
}

/** What `scripts/noto_emoji.py` writes beside the emoji fonts it builds. */
interface EmojiManifest {
  colrv1: string;
  sbix: string;
}

const EMOJI_FAMILY = "Noto Color Emoji";

/**
 * The emoji face: Google's COLRv1 build for the browsers that draw COLRv1 (Chromium and Firefox),
 * and its bitmaps as `sbix` for the rest (WebKit), which `tech()` lets each browser choose
 * between without fetching the other. Each is one whole file: WebKit draws a sequence (a family,
 * a skin tone, a flag) apart when its characters come from different `unicode-range` slices.
 */
function emojiFace(): { css: string; fontFamily: string } {
  const manifest = JSON.parse(
    readFileSync(join(emojiDirectory, "manifest.json"), "utf8"),
  ) as EmojiManifest;
  const directory = relative(generated, emojiDirectory).split("\\").join("/");
  const css = `@font-face {
  font-family: ${quoted(EMOJI_FAMILY)};
  font-style: normal;
  font-display: swap;
  font-weight: 400;
  src:
    url("${directory}/${manifest.colrv1}") format("woff2") tech(color-COLRv1),
    url("${directory}/${manifest.sbix}") format("woff2") tech(color-sbix);
}`;
  return { css, fontFamily: EMOJI_FAMILY };
}

function quoted(name: string): string {
  return `"${name}"`;
}

const HEADER = "/* Written by fallbackFonts.ts when Vite starts; do not edit. */";

function generate(): { faces: string; stacks: string } {
  const scriptFaces = scripts.map(faces);
  const hanFaces = Object.fromEntries(HAN.map((id) => [id, faces(han[id])])) as Record<
    Han,
    ReturnType<typeof faces>
  >;
  const emojiFaces = emojiFace();
  const hanStack = (first?: Han) =>
    [...(first === undefined ? [] : [first]), ...HAN.filter((id) => id !== first)]
      .map((id) => quoted(hanFaces[id].fontFamily))
      .join(", ");
  return {
    faces: [
      HEADER,
      emojiFaces.css,
      ...scriptFaces.map((face) => face.css),
      ...HAN.map((id) => hanFaces[id].css),
      "",
    ].join("\n"),
    stacks: [
      HEADER,
      `:root {
  --font-emoji: ${quoted(emojiFaces.fontFamily)};
  --font-scripts: ${scriptFaces.map((face) => quoted(face.fontFamily)).join(", ")};
  --font-han: ${hanStack()};
}`,
      ...HAN.map(
        (id) =>
          `${HAN_LANGUAGES[id].map((language) => `:root:lang(${language})`).join(", ")} {
  --font-han: ${hanStack(id)};
}`,
      ),
      "",
    ].join("\n"),
  };
}

/** Writes `css` to `name`, unless it already holds it, which would reload a dev server for nothing. */
function write(name: string, css: string): void {
  const path = join(generated, name);
  let current: string | undefined;
  try {
    current = readFileSync(path, "utf8");
  } catch {
    current = undefined;
  }
  if (current !== css) {
    writeFileSync(path, css);
  }
}

/**
 * Writes the fallback faces' stylesheets before anything imports them. The fonts' licences
 * travel with the build among every package's (`attributions/plugin.ts`).
 */
export function fallbackFonts(): Plugin {
  return {
    name: "aspen-fallback-fonts",
    configResolved() {
      const { faces, stacks } = generate();
      mkdirSync(generated, { recursive: true });
      write("fontFaces.css", faces);
      write("fontStacks.css", stacks);
    },
  };
}
