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
}) => {
  test.skip(isMobile, "the same components serve phones; this checks what they hold");
  await signInToWorld(page, withDeveloperMode);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();

  // A message's actions end with its ID.
  const own = page.locator("article").filter({ hasText: ownText }).last();
  await own.hover();
  const actions = own.getByRole("group", { name: "Message actions" }).getByRole("button");
  await expect(actions.last()).toHaveAccessibleName("Copy message ID");
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await actions.last().click();
  await expect(actions.last()).toHaveAccessibleName("Copied message ID");
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toBe(await own.getAttribute("data-message-id"));

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
