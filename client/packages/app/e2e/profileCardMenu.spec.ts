import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * A person's card opens their menu, which is how a touch screen reaches it, against the stubbed
 * world in `world.ts`, where the caller may remove and ban the bot Helper.
 */

test("a person's card opens everything else that can be done to them", async ({
  page,
  isMobile,
}) => {
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  if (isMobile) {
    await page.getByRole("button", { name: "Show members" }).first().click();
  }
  await page
    .getByRole("complementary", { name: "Members" })
    .or(page.getByRole("dialog", { name: "Members" }))
    .getByRole("button", { name: /^Show profile of Helper/ })
    .click();
  const card = page.getByRole("dialog", { name: "Profile of Helper" });
  await card.getByRole("button", { name: "Actions for Helper" }).click();
  const menu = page.getByRole("menu", { name: "Actions for Helper" });
  await expect(menu.getByRole("menuitem", { name: "Remove from community" })).toBeVisible();
  // The card it was opened from is not offered again.
  await expect(menu.getByRole("menuitem", { name: "Profile", exact: true })).toHaveCount(0);
});
