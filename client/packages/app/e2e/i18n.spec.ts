import { expect, test, type Page } from "@playwright/test";
import { accented, mirrored } from "../src/i18n/pseudo";
import { signInToWorld } from "./world";

/**
 * The language the app shows: the platform's by default, or one chosen in Settings, which the
 * document, the controls, and every request to the server follow. The pseudo-locales stand in
 * for real translations.
 */

async function chooseLanguage(page: Page, name: string) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: /Show Aspen in/ }).click();
  await page.getByRole("option", { name }).click();
  // The list closes before the next key reaches the dialog.
  await expect(page.getByRole("listbox")).toHaveCount(0);
}

test("a language chosen in Settings shows at once and is asked of the server", async ({ page }) => {
  await signInToWorld(page);
  await chooseLanguage(page, "Accented English (for testing)");
  await expect(page.locator("html")).toHaveAttribute("lang", "en-XA");
  await expect(page.getByRole("dialog", { name: accented("Settings") })).toBeVisible();
  await page.keyboard.press("Escape");
  const asked = page.waitForRequest(
    (request) => request.url().includes("/communities/") && request.url().endsWith("/invites"),
  );
  await page.getByRole("button", { name: accented("Invite people") }).click();
  expect((await asked).headers()["accept-language"]).toBe("en-XA");
});

test("a right-to-left language mirrors the layout", async ({ page }) => {
  await signInToWorld(page);
  await chooseLanguage(page, "Mirrored English (for testing right-to-left)");
  await expect(page.locator("html")).toHaveAttribute("dir", "rtl");
  await page.keyboard.press("Escape");
  const rail = page.getByRole("navigation", { name: mirrored("Communities") });
  const box = await rail.boundingBox();
  const width = page.viewportSize()?.width ?? 0;
  expect(box === null ? 0 : box.x + box.width).toBeGreaterThan(width - 1);
});

test.describe("on a British English browser", () => {
  test.use({ locale: "en-GB" });

  test("the app follows the browser's language, in its own variant", async ({ page }) => {
    const signIn = page.waitForRequest((request) => request.url().endsWith("/auth/login"));
    await signInToWorld(page);
    await expect(page.locator("html")).toHaveAttribute("lang", "en-GB");
    expect((await signIn).headers()["accept-language"]).toBe("en-GB, en");
  });
});
