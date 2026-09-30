import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/** The theme in Settings > Appearance, against the stubbed world in `world.ts`. */

const isDark = (page: Page) =>
  page.evaluate(() => {
    const [r = 0, g = 0, b = 0] = (
      getComputedStyle(document.body).backgroundColor.match(/\d+/g) ?? []
    ).map(Number);
    return r + g + b < 200;
  });

async function openSettings(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("radiogroup", { name: "Theme" });
}

test.use({ colorScheme: "light" });

test("the theme overrides the system's preference until set back to follow it", async ({
  page,
}) => {
  await signInToWorld(page);
  const theme = await openSettings(page);
  await expect(theme.getByRole("radio", { name: "System" })).toBeChecked();
  expect(await isDark(page)).toBe(false);
  await theme.getByText("Dark", { exact: true }).click();
  await expect.poll(() => isDark(page)).toBe(true);
  await page.reload();
  await expect.poll(() => isDark(page)).toBe(true);
  const again = await openSettings(page);
  await expect(again.getByRole("radio", { name: "Dark" })).toBeChecked();
  await again.getByText("System", { exact: true }).click();
  await expect.poll(() => isDark(page)).toBe(false);
});
