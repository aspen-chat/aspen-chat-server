import { expect, test, type Page } from "@playwright/test";
import { ownText, signInToWorld } from "./world";

/**
 * The ID wizard, against the stubbed world in `world.ts`, whose account preferences here turn
 * developer mode on: everything with an id offers to copy it, last in whatever shows it.
 */

async function withDeveloperMode(page: Page) {
  await page.route(/\/api\/v1\/users\/(@|%40)me\/preferences$/, async (route) => {
    if (route.request().method() !== "GET") {
      await route.fallback();
      return;
    }
    await route.fulfill({
      json: { values: { "developer.mode": true }, updatedAt: new Date().toISOString() },
    });
  });
}

test("with developer mode on, IDs are offered for copying, last in each place", async ({
  page,
  isMobile,
  browserName,
}) => {
  test.skip(isMobile, "the same components serve phones; this checks what they hold");
  await signInToWorld(page, withDeveloperMode);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();

  // A message's menu, from the last of its actions, has its ID last before deleting it.
  const own = page.locator("article").filter({ hasText: ownText }).last();
  await own.hover();
  await own.getByRole("button", { name: "More actions" }).click();
  const actions = page.getByRole("menu", { name: "Message actions" }).getByRole("menuitem");
  await expect(actions.last()).toHaveText("Delete message");
  await expect(actions.nth(-2)).toHaveText("Copy message ID");
  // Reading the clipboard back takes a permission only Chromium's driver grants.
  if (browserName === "chromium") {
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  }
  await actions.nth(-2).click();
  if (browserName === "chromium") {
    await expect
      .poll(() => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(await own.getAttribute("data-message-id"));
  }

  // So does a channel's menu.
  await page.getByRole("button", { name: "Options for general" }).first().click();
  const items = page.getByRole("menu", { name: "Options for general" }).getByRole("menuitem");
  await expect(items.last()).toHaveText("Copy channel ID");
  await page.keyboard.press("Escape");

  // And a person's card.
  await own
    .getByRole("button", { name: /Show profile of/ })
    .last()
    .click();
  const card = page.getByRole("dialog", { name: /Profile of|profile/i }).last();
  await expect(card.getByRole("button").last()).toHaveAccessibleName("Copy user ID");
});

test("without developer mode, nothing offers an ID", async ({ page, isMobile }) => {
  test.skip(isMobile, "the same components serve phones");
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("button", { name: /^Copy .* ID$/ })).toHaveCount(0);
});
