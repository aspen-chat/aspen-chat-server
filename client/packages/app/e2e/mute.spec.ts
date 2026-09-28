import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Muting channels and DMs from their menus, against the stubbed world in `world.ts`, where
 * #roadmap and the DM with Bob hold messages the caller has not read.
 */

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("a muted channel is dimmed, never unread, and can be unmuted", async ({ page }) => {
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: "Family, unread" })).toBeVisible();
  await page.getByText("roadmap", { exact: true }).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Options for roadmap" });
  await menu.getByRole("menuitem", { name: "For 1 hour" }).click();
  await expect(page.getByText("roadmap, muted")).toBeAttached();
  await expect(page.getByText("roadmap, unread")).toHaveCount(0);
  // Its unread no longer marks the community.
  await expect(rail(page).getByRole("row", { name: "Family" })).toBeVisible();

  await page.getByRole("button", { name: "Options for roadmap" }).click();
  await expect(menu.getByText(/^Muted until /)).toBeVisible();
  await menu.getByRole("menuitem", { name: "Unmute" }).click();
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: "Family, unread" })).toBeVisible();
});

test("a DM is muted from its options button until unmuted", async ({ page }) => {
  await rail(page).getByRole("link", { name: "Direct messages, unread" }).click();
  await page
    .getByRole("button", { name: "Options for Bob With A Rather Long Display Name" })
    .click();
  const menu = page.getByRole("menu", { name: /^Options for Bob/ });
  await menu.getByRole("menuitem", { name: "Until I unmute it" }).click();
  await expect(page.getByText("Bob With A Rather Long Display Name, muted")).toBeAttached();
  await expect(
    rail(page).getByRole("link", { name: "Direct messages", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: /^Options for Bob/ }).click();
  await expect(menu.getByText("Muted until you unmute it")).toBeVisible();
});
