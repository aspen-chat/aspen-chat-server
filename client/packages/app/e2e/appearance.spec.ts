import { expect, test, type Page } from "@playwright/test";
import { bob, general, signInToWorld } from "./world";

/**
 * Appearance and accessibility, against the stubbed world: the message text size and line
 * spacing are kept with the account and size messages alone; zoom is the desktop shell's, stood
 * in for by its bridge, and a browser is told to use its own; contrast follows the system or the
 * reader's choice; the keyboard moves between messages; and arriving
 * messages are read out where the reader asks.
 */

declare global {
  interface Window {
    zoomSet: number[];
    zoomStep: ((step: 1 | -1 | 0) => void) | null;
  }
}

async function openSettings(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("dialog", { name: "Settings", exact: true });
}

/** Opens #general, from the channel list that shows beside it, or in its place on a phone. */
async function openGeneral(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
}

test("the message text size is kept with the account and sizes messages", async ({ page }) => {
  await signInToWorld(page);
  await openGeneral(page);
  const message = page.locator("[data-message-id] .message-body").first();
  await expect(message).toBeVisible();
  const fontSize = (element: Element) => getComputedStyle(element).fontSize;
  expect(await message.evaluate(fontSize)).toBe("16px");
  const heading = page.getByRole("main").getByRole("heading").first();
  const headingSize = await heading.evaluate(fontSize);

  const settings = await openSettings(page);
  const slider = settings.getByRole("slider", { name: "Message text size" });
  await expect(slider).toHaveAttribute("aria-valuetext", "16px");
  const saved = page.waitForRequest(
    (r) => r.method() === "PATCH" && /\/users\/(@|%40)me\/preferences$/.test(r.url()),
  );
  await slider.focus();
  await page.keyboard.press("End");
  expect((await saved).postDataJSON()).toEqual({ "look.messageTextSize": 24 });
  await expect(slider).toHaveAttribute("aria-valuetext", "24px");
  await page.keyboard.press("Escape");
  await openGeneral(page);

  await expect.poll(() => message.evaluate(fontSize)).toBe("24px");
  // The rest of the app keeps its size.
  expect(await heading.evaluate(fontSize)).toBe(headingSize);
});

test("a browser is told how to use its own zoom", async ({ page }) => {
  await signInToWorld(page);
  const settings = await openSettings(page);
  await expect(settings.getByText("Your browser sets how large Aspen is drawn.")).toBeVisible();
  await expect(settings.getByRole("slider", { name: "Zoom" })).toHaveCount(0);
});

test("the desktop app's zoom steps with the slider and with the keys", async ({
  page,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "").origin;
  await page.addInitScript((server) => {
    localStorage.setItem("aspen.serverUrl", server);
    window.zoomSet = [];
    window.zoomStep = null;
    (window as { aspenDesktop?: unknown }).aspenDesktop = {
      platform: "linux",
      versions: { electron: "0", chrome: "0" },
      zoom: {
        get: () => Promise.resolve(1.25),
        set: (factor: number) => {
          window.zoomSet.push(factor);
          return Promise.resolve();
        },
        onStep: (listener: (step: 1 | -1 | 0) => void) => {
          window.zoomStep = listener;
          return () => {
            window.zoomStep = null;
          };
        },
      },
    };
  }, origin);
  await signInToWorld(page);
  const settings = await openSettings(page);
  const slider = settings.getByRole("slider", { name: "Zoom" });
  await expect(slider).toHaveAttribute("aria-valuetext", "125%");
  await expect(settings.getByText("Ctrl + and Ctrl − change it too")).toBeVisible();

  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await expect(slider).toHaveAttribute("aria-valuetext", "150%");
  expect(await page.evaluate(() => window.zoomSet)).toEqual([1.5]);

  // Ctrl − in the main process arrives as a step down, and Ctrl 0 as a step back to normal.
  await page.evaluate(() => {
    window.zoomStep?.(-1);
  });
  await expect(slider).toHaveAttribute("aria-valuetext", "125%");
  await page.evaluate(() => {
    window.zoomStep?.(0);
  });
  await expect(slider).toHaveAttribute("aria-valuetext", "100%");
  expect(await page.evaluate(() => window.zoomSet)).toEqual([1.5, 1.25, 1]);
});

test("the line spacing is kept with the account and spaces messages", async ({ page }) => {
  await signInToWorld(page);
  const settings = await openSettings(page);
  const slider = settings.getByRole("slider", { name: "Line spacing" });
  await expect(slider).toHaveAttribute("aria-valuetext", "Normal");
  const saved = page.waitForRequest(
    (r) => r.method() === "PATCH" && /\/users\/(@|%40)me\/preferences$/.test(r.url()),
  );
  await slider.focus();
  await page.keyboard.press("End");
  expect((await saved).postDataJSON()).toEqual({ "look.messageSpacing": 1.4 });
  await expect(slider).toHaveAttribute("aria-valuetext", "Wider");
  await page.keyboard.press("Escape");
  await openGeneral(page);
  const message = page.locator("[data-message-id] .message-body").first();
  // 16px text at 1.5 lines, 1.4 times as far apart.
  await expect
    .poll(() => message.evaluate((element) => getComputedStyle(element).lineHeight))
    .toBe("33.6px");
});

test("contrast follows the system, and the reader's choice over it", async ({ page }) => {
  await page.emulateMedia({ contrast: "more" });
  await signInToWorld(page);
  const root = page.locator("html");
  await expect(root).toHaveAttribute("data-contrast", "more");
  const settings = await openSettings(page);
  const contrast = settings.getByRole("radiogroup", { name: "Contrast" });
  await contrast.getByText("Standard", { exact: true }).click();
  await expect(root).not.toHaveAttribute("data-contrast", "more");
  await contrast.getByText("More", { exact: true }).click();
  await page.emulateMedia({ contrast: "no-preference" });
  await expect(root).toHaveAttribute("data-contrast", "more");
  expect(await page.evaluate(() => localStorage.getItem("aspen.contrast"))).toBe("more");
});

test("the keyboard moves between messages, the list one stop in the tab order", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a phone's list is moved through by touch");
  await signInToWorld(page);
  await openGeneral(page);
  const list = page.locator("[data-message-list]");
  const stops = list.locator('[data-message-row][tabindex="0"]');
  await expect(stops).toHaveCount(1);
  const rows = list.locator("[data-message-row]");
  const count = await rows.count();
  // The stop is the newest message until another is chosen.
  await expect(stops).toHaveAttribute(
    "data-message-row",
    (await rows.nth(count - 1).getAttribute("data-message-row")) ?? "",
  );
  await stops.focus();
  await page.keyboard.press("ArrowUp");
  await expect(rows.nth(count - 2)).toBeFocused();
  await expect(rows.nth(count - 2)).toHaveAttribute("tabindex", "0");
  await page.keyboard.press("Home");
  await expect(rows.first()).toBeFocused();
  await page.keyboard.press("End");
  await expect(rows.nth(count - 1)).toBeFocused();
  await expect(stops).toHaveCount(1);
});

test("arriving messages are read out where the reader asks", async ({ page }) => {
  const publish = await signInToWorld(page);
  const settings = await openSettings(page);
  await settings.getByText("Read out new messages", { exact: true }).click();
  await expect(settings.getByRole("checkbox", { name: /Read out new messages/ })).toBeChecked();
  await page.keyboard.press("Escape");
  await openGeneral(page);
  publish({
    serverEvent: "message",
    type: "create",
    id: "0190f0a0-0000-7000-8001-000000000977",
    channelId: general,
    author: bob,
    content: "A quick hello",
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    linkedMessages: [],
    alteredBy: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [], roles: [], everyone: false },
  });
  await expect(page.locator('[aria-live="polite"][aria-relevant="additions"]')).toContainText(
    "A quick hello",
  );
});
