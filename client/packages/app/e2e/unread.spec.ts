import { expect, test, type Page } from "@playwright/test";
import { dm, dmMessageId, lastReadText, signInToWorld } from "./world";

/**
 * Unread channels and the "New Messages" line, against the stubbed world in `world.ts`, where
 * #general, #roadmap, and the DM with Bob hold messages the caller has not read.
 */

const channelList = (page: Page) => page.getByRole("grid", { name: "Channels" }).first();
const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("unread channels, communities, and DMs are marked until they are read", async ({ page }) => {
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: "Family, unread" })).toBeVisible();
  const dms = rail(page).getByRole("link", { name: "Direct messages, unread" });
  await expect(dms).toBeVisible();

  await dms.click();
  const bob = page.getByRole("link", { name: /Bob With A Rather Long Display Name, unread/ });
  await expect(bob).toBeVisible();
  const report = page.waitForRequest(
    (request) =>
      request.method() === "PUT" && request.url().includes(`/channels/${dm}/read-states/@me`),
  );
  await bob.click();
  // The newest message on screen is read, and reported.
  expect((await report).postDataJSON()).toEqual({ lastRead: dmMessageId });
  const back = page.getByRole("link", { name: "Back to direct messages" });
  if (await back.isVisible()) {
    await back.click();
  }
  await expect(
    rail(page).getByRole("link", { name: "Direct messages", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Bob With A Rather Long Display Name", exact: true }),
  ).toBeVisible();
});

test("the New Messages line sits under the last message read, and stays while reading", async ({
  page,
}) => {
  await channelList(page).getByText("general", { exact: true }).click();
  const line = page.getByRole("separator", { name: "New Messages" });
  await expect(line).toBeVisible();
  // Directly under the last message read.
  const above = line.locator("xpath=preceding-sibling::*[1]");
  await expect(above).toContainText(lastReadText);
  // Reading moved the position past it, and the line stays where it was.
  await page.waitForRequest((request) => request.url().includes("/read-states/@me"));
  await expect(line).toBeVisible();
});
