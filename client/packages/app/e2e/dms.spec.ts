import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/** The DM view, against the stubbed world in `world.ts`. */

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("the DM list keeps the signed-in user's controls at its foot", async ({ page }) => {
  await page
    .getByRole("navigation", { name: "Communities" })
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await expect(page.getByRole("heading", { name: "Direct messages" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit profile" })).toBeVisible();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Settings", exact: true })).toBeVisible();
});
