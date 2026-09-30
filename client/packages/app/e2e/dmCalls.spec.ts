import { expect, test, type Page } from "@playwright/test";
import { bob, dm, signInToWorld } from "./world";

/**
 * Calls in DMs, against the stubbed world in `world.ts`, where the caller has a DM with Bob.
 * The world has no voice server, so these check what a call under way shows; joining one is
 * checked against real servers by hand.
 */

const session = "0190f0a0-0000-7000-8000-000000000071";
const bobName = "Bob With A Rather Long Display Name";

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test("a call under way in a DM shows on its row and above its messages", async ({ page }) => {
  const publish = await signInToWorld(page);
  await rail(page)
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  const row = page.getByRole("navigation", { name: "Direct messages" }).getByRole("link", {
    name: new RegExp(bobName),
  });
  await expect(row.getByRole("img", { name: "Call under way" })).toHaveCount(0);

  publish({
    serverEvent: "voiceSession",
    type: "create",
    id: session,
    channel: dm,
    voiceServer: "0190f0a0-0000-7000-8000-000000000072",
    createdAt: new Date().toISOString(),
  });
  publish({
    serverEvent: "voiceParticipant",
    type: "create",
    session,
    channel: dm,
    user: bob,
    joinedAt: new Date().toISOString(),
    muted: false,
    deafened: false,
    sharingScreen: false,
  });
  await expect(row.getByRole("img", { name: "Call under way" })).toBeVisible();
  await row.click();
  const call = page.getByRole("region", { name: "Call", exact: true });
  await expect(call.locator(`[data-voice-tile="${bob}"]`)).toBeVisible();
  await expect(call.getByRole("button", { name: "Join the call" })).toBeVisible();
  // The header offers the same.
  await expect(page.locator("header").getByRole("button", { name: "Join the call" })).toBeVisible();
});

test("a person's card offers to call them, and a bot's does not", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  await signInToWorld(page);
  const members = page.getByRole("complementary", { name: "Members" });
  await members.getByRole("button", { name: `Show profile of ${bobName}` }).click();
  const card = page.getByRole("dialog", { name: `Profile of ${bobName}` });
  await card.getByRole("button", { name: `Call ${bobName}` }).hover();
  await expect(page.getByRole("tooltip")).toHaveText(`Call ${bobName}`);
  // The first Escape closes the tooltip, the second the card.
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await expect(card).toHaveCount(0);
  await members.getByRole("button", { name: "Show profile of Helper" }).click();
  await expect(
    page.getByRole("dialog", { name: "Profile of Helper" }).getByRole("button", { name: /^Call / }),
  ).toHaveCount(0);
});
