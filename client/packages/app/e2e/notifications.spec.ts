import { expect, test, type Page } from "@playwright/test";
import { bob, me, roadmap, signInToWorld, type Publish } from "./world";

/**
 * Notification settings and what they bring, against the stubbed world: a channel's menu sets
 * what it tells of, and a message it tells of while the user looks elsewhere shows the system's
 * notification (a stand-in for the browser's, which records what it is asked to show) and plays
 * the notification sound.
 */

declare global {
  interface Window {
    shownNotifications: { title: string; body: string }[];
    soundsPlayed: number;
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.shownNotifications = [];
    window.soundsPlayed = 0;
    class StandIn {
      static permission = "granted";
      static requestPermission() {
        return Promise.resolve("granted");
      }
      onclick: (() => void) | null = null;
      constructor(title: string, options: { body: string }) {
        window.shownNotifications.push({ title, body: options.body });
      }
      close() {
        return undefined;
      }
    }
    Object.defineProperty(window, "Notification", { value: StandIn });
    HTMLMediaElement.prototype.play = function play() {
      window.soundsPlayed += 1;
      return Promise.resolve();
    };
  });
});

let sent = 900;

/** A new message by Bob in #roadmap, as the stream announces it. */
function post(publish: Publish, content: string, tags: string[] = []) {
  sent += 1;
  publish({
    serverEvent: "message",
    type: "create",
    id: `0190f0a0-0000-7000-8001-${String(sent).padStart(12, "0")}`,
    channelId: roadmap,
    author: bob,
    content,
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: tags, roles: [], everyone: false },
  });
}

async function turnOnSystemNotifications(page: Page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByText("Show system notifications", { exact: true }).click();
  await expect(settings.getByRole("checkbox", { name: /Show system notifications/ })).toBeChecked();
  await page.keyboard.press("Escape");
}

test("a tag elsewhere shows a system notification and plays the sound", async ({ page }) => {
  const publish = await signInToWorld(page);
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await turnOnSystemNotifications(page);
  post(publish, `<@${me}> can you look at this?`, [me]);
  await expect
    .poll(() => page.evaluate(() => window.shownNotifications))
    .toEqual([
      { title: expect.stringContaining("#roadmap (Family)"), body: "@kate can you look at this?" },
    ]);
  expect(await page.evaluate(() => window.soundsPlayed)).toBeGreaterThan(0);

  post(publish, "Just thinking out loud");
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => window.shownNotifications.length)).toBe(1);
});

test("a channel's menu sets what it tells of", async ({ page }) => {
  const publish = await signInToWorld(page);
  await turnOnSystemNotifications(page);
  await page.getByText("roadmap", { exact: true }).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Options for roadmap" });
  await expect(menu.getByRole("menuitemradio", { name: "Default (Only tags)" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  const asked = page.waitForRequest((r) =>
    r.url().endsWith(`/channels/${roadmap}/notification-settings/@me`),
  );
  await menu.getByRole("menuitemradio", { name: "All messages" }).click();
  expect((await asked).postDataJSON()).toEqual({ level: "all" });
  post(publish, "Just thinking out loud");
  await expect.poll(() => page.evaluate(() => window.shownNotifications.length)).toBe(1);

  await page.getByText("roadmap", { exact: true }).click({ button: "right" });
  await expect(menu.getByRole("menuitemradio", { name: "All messages" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await menu.getByRole("menuitemradio", { name: "Nothing" }).click();
  post(publish, `<@${me}> hello?`, [me]);
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => window.shownNotifications.length)).toBe(1);
});
