import { expect, test, type Page } from "@playwright/test";
import { bob, me, roadmap, signInToWorld, submit } from "./world";

/**
 * Drafts, against the stubbed world: what is written in a message box and not sent waits in its
 * channel through going elsewhere, opening a notification, and reloading, and goes once sent.
 */

declare global {
  interface Window {
    openedNotifications: { onclick: (() => void) | null }[];
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.openedNotifications = [];
    class StandIn {
      static permission = "granted";
      static requestPermission() {
        return Promise.resolve("granted");
      }
      onclick: (() => void) | null = null;
      constructor() {
        window.openedNotifications.push(this);
      }
      close() {
        return undefined;
      }
    }
    Object.defineProperty(window, "Notification", { value: StandIn });
  });
});

/** Opens a channel of the community, by way of the channel list on a phone. */
async function openChannel(page: Page, name: string) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText(name, { exact: true }).click();
}

const box = (page: Page) => page.getByRole("textbox", { name: "Message" });

test("a draft waits in its channel while the reader is elsewhere, and after a reload", async ({
  page,
}) => {
  await signInToWorld(page);
  await openChannel(page, "general");
  await box(page).fill("half a thought");
  await openChannel(page, "roadmap");
  await expect(box(page)).toHaveValue("");
  await openChannel(page, "general");
  await expect(box(page)).toHaveValue("half a thought");
  await page.reload();
  await expect(box(page)).toHaveValue("half a thought");
});

test("opening a notification keeps what was being typed, to the last keystroke", async ({
  page,
}) => {
  const publish = await signInToWorld(page);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByText("Show system notifications", { exact: true }).click();
  await page.keyboard.press("Escape");
  await openChannel(page, "general");
  publish({
    serverEvent: "message",
    type: "create",
    id: "0190f0a0-0000-7000-8001-000000000950",
    channelId: roadmap,
    author: bob,
    content: `<@${me}> a quick question`,
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [me], roles: [], everyone: false },
  });
  await expect.poll(() => page.evaluate(() => window.openedNotifications.length)).toBe(1);
  await box(page).fill("I was just saying");
  // Opened the instant after typing, before the draft's pause has passed.
  await page.evaluate(() => window.openedNotifications[0]?.onclick?.());
  await expect(box(page)).toHaveAttribute("aria-placeholder", "Message #roadmap");
  await openChannel(page, "general");
  await expect(box(page)).toHaveValue("I was just saying");
});

test("a notification of the channel being typed in keeps the draft there", async ({ page }) => {
  const publish = await signInToWorld(page);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByText("Show system notifications", { exact: true }).click();
  await page.keyboard.press("Escape");
  await openChannel(page, "roadmap");
  // Looking elsewhere, as a notification needs.
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
  });
  publish({
    serverEvent: "message",
    type: "create",
    id: "0190f0a0-0000-7000-8001-000000000951",
    channelId: roadmap,
    author: bob,
    content: `<@${me}> are you there?`,
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [me], roles: [], everyone: false },
  });
  await expect.poll(() => page.evaluate(() => window.openedNotifications.length)).toBe(1);
  await box(page).fill("yes, one moment");
  await page.evaluate(() => window.openedNotifications[0]?.onclick?.());
  await expect(page).toHaveURL(/\/messages\//);
  await expect(box(page)).toHaveValue("yes, one moment");
});

test("sending a message clears its draft", async ({ page }) => {
  await signInToWorld(page);
  await openChannel(page, "general");
  await box(page).fill("on its way");
  await submit(page);
  await expect(box(page)).toHaveValue("");
  await openChannel(page, "roadmap");
  await openChannel(page, "general");
  await expect(box(page)).toHaveValue("");
  await page.reload();
  await expect(box(page)).toHaveValue("");
});
