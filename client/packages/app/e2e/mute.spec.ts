import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Muting channels and DMs from their menus, against the stubbed world in `world.ts`, where
 * #roadmap and the DM with Bob hold messages the caller has not read.
 */

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });
const muteMenu = (page: Page) => page.getByRole("menu", { name: "Mute" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("a muted channel is dimmed, never unread, and can be unmuted", async ({ page, isMobile }) => {
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: "Family, unread" })).toBeVisible();
  await page.getByText("roadmap", { exact: true }).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Options for roadmap" });
  await menu.getByRole("menuitem", { name: "Mute" }).click();
  await muteMenu(page).getByRole("menuitem", { name: "For 1 hour" }).click();
  await expect(page.getByText("roadmap, muted")).toBeAttached();
  await expect(page.getByText("roadmap, unread")).toHaveCount(0);
  // Its unread no longer marks the community.
  await expect(rail(page).getByRole("row", { name: "Family" })).toBeVisible();
  // Its bell's tooltip says until when. A phone has no hover, and the options button's touch
  // area covers the bell there; its menu says the same.
  if (!isMobile) {
    await page.getByRole("img", { name: /^Muted until / }).hover();
    await expect(page.getByRole("tooltip")).toHaveText(/^Muted until /);
  }

  await page.getByRole("button", { name: "Options for roadmap" }).click();
  // The submenu's own item says until when.
  await menu.getByRole("menuitem", { name: /^Muted until / }).click();
  await muteMenu(page).getByRole("menuitem", { name: "Unmute" }).click();
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: "Family, unread" })).toBeVisible();
});

test("a DM is muted from its options button until unmuted", async ({ page, isMobile }) => {
  await rail(page).getByRole("link", { name: "Direct messages, unread" }).click();
  await page
    .getByRole("button", { name: "Options for Bob With A Rather Long Display Name" })
    .click();
  const menu = page.getByRole("menu", { name: /^Options for Bob/ });
  await menu.getByRole("menuitem", { name: "Mute" }).click();
  await muteMenu(page).getByRole("menuitem", { name: "Until I unmute it" }).click();
  await expect(page.getByText("Bob With A Rather Long Display Name, muted")).toBeAttached();
  // Its bell, beside the options button, says until when in its tooltip.
  if (!isMobile) {
    await page.getByRole("img", { name: "Muted until you unmute it" }).hover();
    await expect(page.getByRole("tooltip")).toHaveText("Muted until you unmute it");
  }
  await expect(
    rail(page).getByRole("link", { name: "Direct messages", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: /^Options for Bob/ }).click();
  await expect(menu.getByRole("menuitem", { name: "Muted until you unmute it" })).toBeVisible();
});
