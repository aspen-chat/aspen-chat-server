import { expect, test, type Page } from "@playwright/test";
import { community, helper, signInToWorld } from "./world";

/**
 * Bots, against the stubbed world in `world.ts`, where Kate owns Helper, a bot in the Family
 * community, and organises the community, which lets her add bots.
 */

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

/** Opens Settings from wherever the user footer is. */
async function openSettings(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("dialog", { name: "Settings", exact: true });
}

test("a bot is marked as one in the member list and on its card", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  const members = page.getByRole("complementary", { name: "Members" });
  const row = members.getByRole("button", { name: "Show profile of Helper" });
  await expect(row.getByText("Bot", { exact: true })).toBeVisible();
  await row.click();
  const card = page.getByRole("dialog", { name: /Helper/ });
  await expect(card.getByText("Bot", { exact: true })).toBeVisible();
  await expect(card.getByText("Made by Kate")).toBeVisible();
});

test("developer mode shows the user's bots, makes one, and shows its token once", async ({
  page,
}) => {
  const settings = await openSettings(page);
  await expect(settings.getByRole("button", { name: "Your bots" })).toHaveCount(0);
  await settings.getByText("Developer mode", { exact: true }).last().click();
  await settings.getByRole("button", { name: "Your bots" }).click();
  const bots = page.getByRole("dialog", { name: "Your bots" });
  await expect(bots.getByRole("list", { name: "Your bots" }).getByText("Helper")).toBeVisible();
  await expect(bots.getByRole("textbox", { name: "Link to add it" })).toHaveValue(
    new RegExp(`/bots/${helper}/add$`),
  );
  await bots.getByLabel("Username").fill("greeter");
  await bots.getByRole("button", { name: "Make bot" }).click();
  await expect(bots.getByRole("textbox", { name: "greeter's token" })).toHaveValue(
    "aspenbot_example",
  );
  await expect(bots.getByRole("list", { name: "Your bots" }).getByText("greeter")).toBeVisible();
});

test("a bot's link adds it to a community with the permissions it suggests", async ({ page }) => {
  await page.goto(`/bots/${helper}/add?permissions=sendMessages,addReactions`);
  await expect(page.getByRole("heading", { name: "Add Helper to a community" })).toBeVisible();
  const permissions = page.getByRole("group", { name: "Permissions to give it" });
  await expect(permissions.getByRole("checkbox", { name: /Send messages/ })).toBeChecked();
  await permissions.getByText("Add reactions", { exact: true }).click();
  const request = page.waitForRequest(
    (r) => r.method() === "PUT" && r.url().endsWith(`/communities/${community}/members/${helper}`),
  );
  await page.getByRole("button", { name: "Add bot" }).click();
  expect((await request).postDataJSON()).toEqual({ permissions: ["sendMessages"] });
  await expect(page).toHaveURL(new RegExp(`/communities/${community}`));
});
