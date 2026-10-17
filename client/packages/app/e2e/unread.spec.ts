import { expect, test, type Page } from "@playwright/test";
import {
  dm,
  dmMessageId,
  general,
  ideas,
  lastReadText,
  longPress,
  signInToWorld,
  type Publish,
} from "./world";
import { messageId } from "./world/fixtures";

/**
 * Unread channels and the "New Messages" line, against the stubbed world in `world.ts`, where
 * #general, #roadmap, and the DM with Bob hold messages the caller has not read.
 */

const channelList = (page: Page) => page.getByRole("grid", { name: "Channels" }).first();
const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

/** Twice `READ_REPORT_MS`, long enough for any read the list made to have been reported. */
const REPORTED_MS = 2_000;

let publish: Publish;

test.beforeEach(async ({ page }) => {
  publish = await signInToWorld(page);
});

/** Opens `name` from the channel list, leaving the channel open first where it covers the list. */
async function openChannel(page: Page, name: string) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("row", { name, exact: true }).getByText(name, { exact: true }).click();
}

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
  // The row is a link, or a button to Bob's card while his DM is open beside the list.
  const list = page.getByRole("navigation", { name: "Direct messages" });
  await expect(
    list.getByText("Bob With A Rather Long Display Name", { exact: true }),
  ).toBeVisible();
  await expect(list.getByText(/, unread$/)).toHaveCount(0);
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

test("marking a message unread keeps the channel unread while it stays open", async ({
  page,
  isMobile,
}) => {
  const reports: unknown[] = [];
  page.on("request", (request) => {
    if (
      request.method() === "PUT" &&
      request.url().includes(`/channels/${general}/read-states/@me`)
    ) {
      reports.push(request.postDataJSON());
    }
  });
  await channelList(page).getByText("general", { exact: true }).click();
  await expect.poll(() => reports).toContainEqual({ lastRead: messageId(207) });

  const message = page.locator("article").filter({ hasText: lastReadText }).last();
  if (isMobile) {
    await longPress(page, message.locator(".message-body"));
    await page
      .getByRole("dialog", { name: "Message actions" })
      .getByRole("button", { name: "Mark unread" })
      .tap();
  } else {
    await message.hover();
    await message.getByRole("button", { name: "Mark unread" }).click();
  }
  await expect.poll(() => reports).toContainEqual({ unreadFrom: messageId(204) });
  await expect(page.getByText("Marked unread")).toBeVisible();
  // The line moves above the message marked, and the channel stays unread while it is shown.
  const line = page.getByRole("separator", { name: "New Messages" });
  await expect(line.locator("xpath=following-sibling::*[1]")).toContainText(lastReadText);
  const marked = reports.length;
  await page.waitForTimeout(REPORTED_MS);
  expect(reports.slice(marked)).toEqual([]);

  // Opening it again reads it.
  await openChannel(page, "ideas");
  await openChannel(page, "general");
  await expect.poll(() => reports.slice(marked)).toContainEqual({ lastRead: messageId(207) });
});

test("a read channel is marked unread from its menu, from its newest message", async ({ page }) => {
  const body = page.waitForRequest(
    (request) =>
      request.method() === "PUT" && request.url().includes(`/channels/${ideas}/read-states/@me`),
  );
  await page
    .getByRole("row", { name: "ideas", exact: true })
    .getByText("ideas", { exact: true })
    .click({ button: "right" });
  await page.getByRole("menuitem", { name: "Mark unread" }).click();
  expect((await body).postDataJSON()).toEqual({ unreadNewest: true });
  publish({
    serverEvent: "channelMarkedUnread",
    channel: ideas,
    lastRead: messageId(229),
    lastMessage: messageId(230),
    mentions: 0,
  });
  await expect(page.getByText("ideas, unread")).toBeAttached();
});
