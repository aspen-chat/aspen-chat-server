import { expect, test, type Page } from "@playwright/test";
import { reactedText, signInToWorld, singleReactions } from "./world";

/**
 * Reactions under a message, against the stubbed world in `world.ts`, where Bob's "Sounds
 * good" has 👍 seven times (the caller among them), 🎉 six times, and twenty emoji once each.
 */

const message = (page: Page) => page.locator("article").filter({ hasText: reactedText }).last();
const chips = (page: Page) => message(page).getByRole("list", { name: "Reactions" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(chips(page)).toBeVisible();
});

test("the most popular reactions come first, and only twenty show", async ({ page }) => {
  const items = chips(page).getByRole("listitem");
  // Twenty emoji, the rest counted, and the add control.
  await expect(items).toHaveCount(22);
  await expect(items.nth(0)).toHaveText("👍7");
  await expect(items.nth(1)).toHaveText("🎉6");
  // Ties keep the order the emoji were first used in.
  await expect(items.nth(2)).toHaveText(`${singleReactions[0] ?? ""}1`);
  await expect(items.nth(20)).toHaveText("+2");
  await expect(chips(page).getByRole("button", { name: "Add a reaction" })).toBeVisible();
});

test("a reaction's tooltip names the first four and counts the rest", async ({ page }) => {
  await chips(page).getByRole("button", { name: "Remove your 👍" }).hover();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Kate, Bob With A Rather Long Display Name … and 5 more reacted with 👍",
  );
});

test("the rest open in a list of every reaction, with who reacted", async ({ page }) => {
  await chips(page).getByRole("button", { name: "2 more reactions" }).click();
  const dialog = page.getByRole("dialog", { name: "Reactions" });
  const tabs = dialog.getByRole("tab");
  await expect(tabs).toHaveCount(22);
  await expect(tabs.nth(0)).toHaveAccessibleName("👍, 7");
  await expect(dialog.getByRole("tabpanel").getByRole("listitem")).toHaveText([
    "KKate",
    "BNBob With A Rather Long Display Name",
  ]);
  await tabs.nth(1).click();
  await expect(dialog.getByRole("tabpanel").getByRole("listitem")).toHaveCount(1);
});

test("adding a reaction from the chips takes one tap", async ({ page }) => {
  await chips(page).getByRole("button", { name: "Add a reaction" }).click();
  await expect(page.getByRole("dialog", { name: "Add a reaction" })).toBeVisible();
});

test("the message's own actions open the list of reactions", async ({ page, isMobile }) => {
  if (isMobile) {
    await message(page).locator(".message-body").tap();
  } else {
    await message(page).hover();
  }
  await message(page).getByRole("button", { name: "View reactions" }).click();
  await expect(page.getByRole("dialog", { name: "Reactions" })).toBeVisible();
});

test("a right click on a chip shows who reacted with it", async ({ page, isMobile }) => {
  test.skip(isMobile, "a phone presses long instead");
  await chips(page).getByRole("button", { name: "React with 🎉" }).click({ button: "right" });
  const dialog = page.getByRole("dialog", { name: "Reactions" });
  await expect(dialog.getByRole("tab", { selected: true })).toHaveAccessibleName("🎉, 6");
});

test("a long press on a chip shows who reacted, and leaves the reaction be", async ({
  page,
  isMobile,
}) => {
  test.skip(!isMobile, "a long press is a touch screen's");
  let reacted = false;
  page.on("request", (request) => {
    if (request.url().includes("/reactions/") && request.method() !== "GET") {
      reacted = true;
    }
  });
  const chip = chips(page).getByRole("button", { name: "Remove your 👍" });
  await chip.dispatchEvent("pointerdown", { pointerType: "touch", isPrimary: true });
  await page.waitForTimeout(700);
  await chip.dispatchEvent("pointerup", { pointerType: "touch", isPrimary: true });
  const dialog = page.getByRole("dialog", { name: "Reactions" });
  await expect(dialog.getByRole("tab", { selected: true })).toHaveAccessibleName("👍, 7");
  expect(reacted).toBe(false);
});
