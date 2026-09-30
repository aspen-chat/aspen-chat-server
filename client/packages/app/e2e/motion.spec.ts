import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Animation speed, against the stubbed world: a slider in Settings sets how fast the app moves,
 * kept with the account; at the far left nothing animates; a device set to reduce motion fades
 * without travel.
 */

declare global {
  interface Window {
    animationsStarted: string[];
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.animationsStarted = [];
    document.addEventListener("animationstart", (event) => {
      window.animationsStarted.push(event.animationName);
    });
  });
});

async function openSettings(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("dialog", { name: "Settings", exact: true });
}

test("dialogs grow into place, and the speed slider is kept with the account", async ({ page }) => {
  await signInToWorld(page);
  const settings = await openSettings(page);
  await expect
    .poll(() => page.evaluate(() => window.animationsStarted))
    .toEqual(expect.arrayContaining(["aspen-arrive", "aspen-fade-in"]));

  const slider = settings.getByRole("slider", { name: "Animation speed" });
  await expect(slider).toHaveAttribute("aria-valuetext", "100%");
  const saved = page.waitForRequest(
    (r) => r.method() === "PATCH" && /\/users\/(@|%40)me\/preferences$/.test(r.url()),
  );
  await slider.focus();
  await page.keyboard.press("End");
  expect((await saved).postDataJSON()).toEqual({ "look.motionSpeed": 2 });
  await expect(slider).toHaveAttribute("aria-valuetext", "200%");
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--motion-scale")),
    )
    .toBe("0.5");

  // At the far left, nothing animates.
  await page.keyboard.press("Home");
  await expect(slider).toHaveAttribute("aria-valuetext", "0%");
  await expect(page.locator("html")).toHaveAttribute("data-motion", "off");
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    window.animationsStarted = [];
  });
  await openSettings(page);
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.animationsStarted)).toEqual([]);
});

test("a device that asks to reduce motion fades without travel", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await signInToWorld(page);
  const distance = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--motion-distance").trim(),
  );
  expect(distance).toBe("0px");
});
