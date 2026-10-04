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
        "Noto Color Emoji",
        "Noto Sans Variable",
      ]),
    );
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
