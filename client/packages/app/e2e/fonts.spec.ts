import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * The fonts in Settings, against the stubbed world in `world.ts`. The font added is Noto Sans
 * Ogham (`fixtures/`, under the OFL beside it), chosen for being a real face of a few kilobytes.
 */

const OGHAM = fileURLToPath(new URL("fixtures/NotoSansOgham-Regular.ttf", import.meta.url));

async function openSettings(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("region", { name: "Fonts" });
}

/** The first family `selector` is drawn in, unquoted, as the browser resolved its `font-family`. */
const firstFamily = (page: Page, selector: string) =>
  page.evaluate((s) => {
    const element = document.querySelector(s);
    return element === null
      ? null
      : getComputedStyle(element).fontFamily.split(",")[0]?.trim().replace(/^"|"$/g, "");
  }, selector);

async function choose(
  page: Page,
  fonts: ReturnType<Page["getByRole"]>,
  label: string,
  option: string,
) {
  await fonts.getByRole("button", { name: new RegExp(`${label}$`) }).click();
  await page.getByRole("option", { name: option }).click();
  // The list closes with an animation; the next select's must not open beside it.
  await expect(page.getByRole("listbox")).toHaveCount(0);
}

test("a font added in Settings draws text and code on this install until it is removed", async ({
  page,
}) => {
  await signInToWorld(page);
  expect(await firstFamily(page, "body")).toBe("Inclusive Sans Variable");
  const fonts = await openSettings(page);
  await expect(fonts.getByText("You haven't added any fonts.")).toBeVisible();

  const chooser = page.waitForEvent("filechooser");
  await fonts.getByRole("button", { name: "Add font files" }).click();
  await (await chooser).setFiles(OGHAM);
  const family = fonts.getByRole("listitem").filter({ hasText: "Noto Sans Ogham" });
  await expect(family).toContainText("1 file");

  await choose(page, fonts, "Text", "Noto Sans Ogham");
  await expect.poll(() => firstFamily(page, "body")).toMatch(/^AspenFont\d+$/);
  await choose(page, fonts, "Code", "Noto Sans Ogham");
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--font-code-chosen")),
    )
    .toMatch(/^"AspenFont\d+"$/);

  // Kept with the install: the files in IndexedDB and the choice beside the palette.
  await page.reload();
  await expect.poll(() => firstFamily(page, "body")).toMatch(/^AspenFont\d+$/);
  const again = await openSettings(page);
  await expect(again.getByRole("button", { name: /Text$/ })).toContainText("Noto Sans Ogham");

  await again.getByRole("button", { name: "Remove Noto Sans Ogham" }).click();
  await expect(again.getByText("You haven't added any fonts.")).toBeVisible();
  await expect(again.getByRole("button", { name: /Text$/ })).toContainText("Inclusive Sans");
  await expect(again.getByRole("button", { name: /Code$/ })).toContainText("Intel One Mono");
  await expect.poll(() => firstFamily(page, "body")).toBe("Inclusive Sans Variable");
});

test("a file that is not a font Aspen can read is refused with the reason", async ({ page }) => {
  await signInToWorld(page);
  const fonts = await openSettings(page);
  const chooser = page.waitForEvent("filechooser");
  await fonts.getByRole("button", { name: "Add font files" }).click();
  await (
    await chooser
  ).setFiles([
    { name: "Packed.woff2", mimeType: "font/woff2", buffer: Buffer.from("wOF2\0\0\0\0\0\0\0\0") },
    { name: "notes.ttf", mimeType: "font/ttf", buffer: Buffer.from("not a font at all") },
  ]);
  const alert = fonts.getByRole("alert");
  await expect(alert).toContainText("Packed.woff2 is a WOFF2 file");
  await expect(alert).toContainText("notes.ttf isn't a font file Aspen can read");
  await expect(fonts.getByText("You haven't added any fonts.")).toBeVisible();
});

test("text Aspen's own faces lack is drawn in the bundled Noto faces", async ({ page }) => {
  await signInToWorld(page);
  await page.evaluate(() => {
    const sample = document.createElement("p");
    // Arabic, Bengali, Thai, Ethiopic, Tibetan, Japanese, an emoji, and Cyrillic.
    sample.textContent = "مرحبا বাংলা สวัสดี ሰላም བཀྲ་ཤིས་ こんにちは 🌲 привет";
    document.body.append(sample);
  });
  const loaded = () =>
    page.evaluate(() => {
      const families = new Set<string>();
      document.fonts.forEach((face) => {
        if (face.status === "loaded") {
          families.add(face.family.replace(/^"|"$/g, ""));
        }
      });
      return [...families];
    });
  await expect
    .poll(loaded)
    .toEqual(
      expect.arrayContaining([
        "Noto Sans Arabic Variable",
        "Noto Sans Bengali Variable",
        "Noto Sans Thai Variable",
        "Noto Sans Ethiopic Variable",
        "Noto Serif Tibetan Variable",
        "Noto Sans SC Variable",
        "Aspen Noto Color Emoji",
        "Noto Sans Variable",
      ]),
    );
});

// A colour font can load and still draw nothing, where the browser does not draw its kind of
// colour glyphs, so this draws emoji in the bundled face and looks at the pixels. A sequence
// (here a family of four) drawn whole takes one advance; drawn apart, four.
test("emoji are drawn in colour, sequences whole", async ({ page }) => {
  await signInToWorld(page);
  const drawn = await page.evaluate(async () => {
    const font = '64px "Aspen Noto Color Emoji"';
    const family = "\u{1F468}‍\u{1F469}‍\u{1F467}‍\u{1F466}";
    await document.fonts.load(font, `\u{1F600}${family}`);
    const canvas = document.createElement("canvas");
    canvas.width = 96;
    canvas.height = 96;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (context === null) {
      throw new Error("no 2D canvas");
    }
    context.font = font;
    context.textBaseline = "middle";
    context.fillText("\u{1F600}", 8, 48);
    const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
    let coloured = 0;
    for (let i = 0; i < data.length; i += 4) {
      const [r = 0, g = 0, b = 0, a = 0] = data.slice(i, i + 4);
      if (a > 128 && Math.max(r, g, b) - Math.min(r, g, b) > 64) {
        coloured += 1;
      }
    }
    const one = context.measureText("\u{1F600}").width;
    return { coloured, sequenceInAdvances: context.measureText(family).width / one };
  });
  expect(drawn.coloured).toBeGreaterThan(500);
  expect(drawn.sequenceInAdvances).toBeCloseTo(1, 1);
});

// Emoji are drawn larger than the text around them (`size-adjust` in `fallbackFonts.ts`), and a
// community's own emoji with them, into the room above and below the line: a line holding either
// is as tall as one holding an emoji at the face's own size, the size the picker and reactions
// draw it at.
test("emoji are drawn larger than the text, in lines no taller", async ({ page }) => {
  await signInToWorld(page);
  const measured = await page.evaluate(async () => {
    const root = getComputedStyle(document.documentElement);
    const scaled = root.getPropertyValue("--font-emoji").trim();
    const own = root.getPropertyValue("--font-emoji-unscaled").trim();
    await Promise.all([
      document.fonts.load(`20px ${scaled}`, "\u{1F600}"),
      document.fonts.load(`20px ${own}`, "\u{1F600}"),
    ]);
    const host = document.createElement("div");
    host.style.cssText = "position: absolute; top: 0; left: 0; font-size: 20px; width: 40em";
    document.body.append(host);
    const line = (lineHeight: string, family: string, html: string) => {
      const block = document.createElement("div");
      block.style.lineHeight = lineHeight;
      block.style.fontFamily = `"Inclusive Sans Variable", ${family}`;
      block.innerHTML = html;
      host.append(block);
      return block;
    };
    const width = (block: HTMLElement) =>
      block.firstElementChild?.getBoundingClientRect().width ?? Number.NaN;
    const glyph = (family: string) => width(line("normal", family, "<span>\u{1F600}</span>"));
    // A custom emoji's box, as `CustomEmojiGlyph` draws its picture and its placeholders.
    const custom = '<span class="custom-emoji inline-block align-text-bottom"></span>';
    const atOwnSize = (html: string) => `<span style="--emoji-scale: 1">${html}</span>`;
    const heights = (lineHeight: string) => ({
      own: line(lineHeight, own, "Ag \u{1F600}").offsetHeight,
      scaled: line(lineHeight, scaled, "Ag \u{1F600}").offsetHeight,
      custom: line(lineHeight, scaled, `Ag ${custom}`).offsetHeight,
      customAtOwnSize: line(lineHeight, scaled, `Ag ${atOwnSize(custom)}`).offsetHeight,
    });
    const pictures =
      width(line("normal", scaled, custom)) / width(line("normal", scaled, atOwnSize(custom)));
    // A reaction's emoji (`EmojiKey`) stays at its own size, glyph and picture alike.
    const unscaled = (html: string) => `<span class="emoji-unscaled">${html}</span>`;
    const reactionGlyphs = width(line("normal", scaled, unscaled("\u{1F600}"))) / glyph(own);
    const reactionPictures =
      width(line("normal", scaled, unscaled(custom))) /
      width(line("normal", scaled, atOwnSize(custom)));
    const result = {
      scale: Number(root.getPropertyValue("--emoji-scale")),
      glyphs: glyph(scaled) / glyph(own),
      pictures,
      reactionGlyphs,
      reactionPictures,
      normal: heights("normal"),
      fixed: heights("1.375"),
    };
    host.remove();
    return result;
  });
  expect(measured.scale).toBeGreaterThan(1);
  expect(measured.glyphs).toBeCloseTo(measured.scale, 2);
  expect(measured.pictures).toBeCloseTo(measured.scale, 2);
  expect(measured.reactionGlyphs).toBeCloseTo(1, 2);
  expect(measured.reactionPictures).toBeCloseTo(1, 2);
  for (const heights of [measured.normal, measured.fixed]) {
    expect(heights.scaled).toBe(heights.own);
    expect(heights.custom).toBe(heights.customAtOwnSize);
  }
});

test("the regional Han face comes first for the page's language", async ({ page }) => {
  await signInToWorld(page);
  const first = () =>
    page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue("--font-han")
        .split(",")[0]
        ?.trim(),
    );
  expect(await first()).toBe('"Noto Sans SC Variable"');
  await page.evaluate(() => {
    document.documentElement.lang = "ja";
  });
  expect(await first()).toBe('"Noto Sans JP Variable"');
  await page.evaluate(() => {
    document.documentElement.lang = "zh-Hant-HK";
  });
  expect(await first()).toBe('"Noto Sans HK Variable"');
});
