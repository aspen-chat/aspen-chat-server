import { expect, test, type Locator, type Page } from "@playwright/test";
import { inviteCode, ownText, pollQuestion, signInToWorld, starterText } from "./world";

/**
 * The app on a phone: a narrow, touch-only screen, where there is no hover and one list or
 * conversation shows at a time. Runs in the phone projects of `playwright.config.ts` against
 * the stubbed world in `world.ts`.
 */

/** The smallest square a fingertip reliably hits, in CSS pixels. */
const TOUCH = 44;

/** Neither the page nor anything in it may make the screen scroll sideways. */
async function expectNoSidewaysScroll(page: Page) {
  const widths = await page.evaluate(() => ({
    page: document.documentElement.scrollWidth,
    screen: window.innerWidth,
  }));
  expect(widths.page).toBeLessThanOrEqual(widths.screen);
}

/**
 * The area a tap lands in: the element's box, or the larger invisible area `tap-target` gives
 * it on a touch screen.
 */
async function touchArea(control: Locator) {
  return control.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const after = getComputedStyle(element, "::after");
    const extra = after.content === "none" || after.content === "" ? null : after;
    return {
      width: Math.max(box.width, extra ? parseFloat(extra.width) : 0),
      height: Math.max(box.height, extra ? parseFloat(extra.height) : 0),
    };
  });
}

async function expectTouchable(control: Locator) {
  await expect(control).toBeVisible();
  const area = await touchArea(control);
  expect(area.width, "touch width").toBeGreaterThanOrEqual(TOUCH);
  expect(area.height, "touch height").toBeGreaterThanOrEqual(TOUCH);
}

/** Drags a finger from `fromY` to `toY`, as a swipe scrolls on a touch screen (Chromium only). */
async function swipe(page: Page, x: number, fromY: number, toY: number) {
  const client = await page.context().newCDPSession(page);
  const steps = 8;
  await client.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x, y: fromY }],
  });
  for (let i = 1; i <= steps; i++) {
    await client.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x, y: fromY + ((toY - fromY) * i) / steps }],
    });
  }
  await client.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await client.detach();
}

const channelList = (page: Page) => page.getByRole("grid", { name: "Channels" }).first();
const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });
const messageWith = (page: Page, text: string) =>
  page.locator("article").filter({ hasText: text }).last();
const actionsOf = (article: Locator) => article.getByRole("group", { name: "Message actions" });

async function openChannel(page: Page, name: string) {
  await channelList(page).getByText(name, { exact: true }).click();
  await expect(page.getByRole("heading", { name })).toBeVisible();
}

test.describe("on a phone", () => {
  test.beforeEach(async ({ page }) => {
    await signInToWorld(page);
  });

  test("a community opens on its channel list, and back from a channel returns to it", async ({
    page,
  }) => {
    await expect(channelList(page)).toBeVisible();
    await expect(rail(page)).toBeVisible();
    await openChannel(page, "general");
    await expect(channelList(page)).toBeHidden();
    await page.getByRole("link", { name: "Back to channels" }).click();
    await expect(channelList(page)).toBeVisible();
    await expect(page.getByRole("heading", { name: "general" })).toBeHidden();
  });

  test("a conversation takes the whole screen, and the rail comes back with the lists", async ({
    page,
  }) => {
    await openChannel(page, "general");
    await expect(rail(page)).toBeHidden();
    const width = await page
      .locator("main")
      .first()
      .evaluate((main) => main.clientWidth);
    expect(width).toBe(page.viewportSize()?.width);
    await page.getByRole("link", { name: "Back to channels" }).click();
    await expect(rail(page)).toBeVisible();
    await rail(page).getByRole("link", { name: "Direct messages" }).click();
    await expect(rail(page)).toBeVisible();
    await page.getByRole("link", { name: /Bob With A Rather Long Display Name/ }).click();
    await expect(rail(page)).toBeHidden();
    await page.getByRole("link", { name: "Back to direct messages" }).click();
    await expect(rail(page)).toBeVisible();
  });

  test("no screen scrolls sideways", async ({ page }) => {
    await expectNoSidewaysScroll(page);
    await openChannel(page, "general");
    await expect(messageWith(page, starterText)).toBeVisible();
    await expectNoSidewaysScroll(page);
    await page.getByRole("link", { name: /2 replies/ }).click();
    await expect(page.getByRole("heading", { name: "Thread" })).toBeVisible();
    await expectNoSidewaysScroll(page);
    await page.getByRole("button", { name: "Close thread" }).click();
    await page.getByRole("link", { name: "Back to channels" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expectNoSidewaysScroll(page);
  });

  test("a message's time stays on one line", async ({ page }) => {
    await openChannel(page, "general");
    const times = page.locator("article time");
    await expect(times.first()).toBeVisible();
    for (const time of await times.all()) {
      const lines = await time.evaluate((element) => {
        const lineHeight = parseFloat(getComputedStyle(element).lineHeight);
        return element.getBoundingClientRect().height / lineHeight;
      });
      expect(lines).toBeLessThan(1.5);
    }
  });

  test("tapping a message shows its actions, and tapping another moves them", async ({ page }) => {
    await openChannel(page, "general");
    const own = messageWith(page, ownText);
    const starter = messageWith(page, starterText);
    await expect(actionsOf(own)).toBeHidden();
    await own.locator(".message-body").tap();
    await expect(actionsOf(own)).toBeVisible();
    await expect(actionsOf(starter)).toBeHidden();
    for (const name of ["Add a reaction", "Edit message", "Delete message"]) {
      const action = own.getByRole("button", { name });
      await expect(action).toBeVisible();
      expect((await action.boundingBox())?.height).toBeGreaterThanOrEqual(40);
    }
    await starter.locator(".message-body").tap();
    await expect(actionsOf(starter)).toBeVisible();
    await expect(actionsOf(own)).toBeHidden();
    await expect(starter.getByRole("button", { name: "Reply in thread" })).toBeVisible();
  });

  test("a link in a message opens on the first tap", async ({ page }) => {
    // Tapping a link focuses its message, which shows the message's actions; they must not
    // move the link out from under the finger before the tap completes.
    await openChannel(page, "general");
    await page.getByRole("link", { name: /2 replies/ }).tap();
    await expect(page.getByRole("heading", { name: "Thread" })).toBeVisible();
  });

  test("reading back through history keeps the message in view still", async ({ page }) => {
    // iOS Safari has no scroll anchoring of its own, so the list keeps the view still itself;
    // anchoring is turned off here so that what is tested is the list, on every engine.
    await page.addStyleTag({ content: "* { overflow-anchor: none !important; }" });
    await openChannel(page, "general");
    const scroller = page.locator("div.overflow-y-auto").filter({ has: page.locator("article") });
    const articles = scroller.locator("article");
    await expect(articles.first()).toBeVisible();
    const before = await articles.count();
    /** The message nearest the top of the view, and how far below that top it sits. */
    const topmost = () =>
      scroller.evaluate((element) => {
        const origin = element.getBoundingClientRect().top;
        for (const article of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
          const box = article.getBoundingClientRect();
          if (box.bottom > origin) {
            return { id: article.dataset.messageId ?? "", offset: box.top - origin };
          }
        }
        return { id: "", offset: 0 };
      });
    const offsetOf = (id: string) =>
      scroller.evaluate((element, messageId) => {
        const article = element.querySelector(`[data-message-id="${messageId}"]`);
        return article === null
          ? Infinity
          : article.getBoundingClientRect().top - element.getBoundingClientRect().top;
      }, id);
    // Near the top, where the previous page is read.
    await scroller.evaluate((element) => {
      element.scrollTop = 400;
    });
    await expect(scroller.getByText("Loading…")).toBeVisible();
    // The reader keeps going while the page is on its way.
    await scroller.evaluate((element) => {
      element.scrollTop -= 150;
    });
    const reading = await topmost();
    await expect.poll(() => articles.count()).toBeGreaterThan(before);
    await expect(scroller.getByText("Loading…")).toBeHidden();
    // Within a pixel: WebKit scrolls in whole pixels.
    expect(Math.abs((await offsetOf(reading.id)) - reading.offset)).toBeLessThanOrEqual(1);
  });

  test("content growing above what is being read does not move it", async ({ page }) => {
    // As a picture does when it loads above the reader: the list holds the message being
    // read where it was, itself, since iOS Safari has no scroll anchoring to do it.
    await openChannel(page, "general");
    const scroller = page.locator("div.overflow-y-auto").filter({ has: page.locator("article") });
    await expect(scroller.locator("article").first()).toBeVisible();
    await scroller.evaluate((element) => {
      element.scrollTop = element.scrollHeight - element.clientHeight - 300;
    });
    // At rest before anything loads; the list leaves a moving view to the reader.
    await page.waitForTimeout(400);
    const reading = await scroller.evaluate((element) => {
      const origin = element.getBoundingClientRect().top;
      for (const article of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const box = article.getBoundingClientRect();
        if (box.bottom > origin) {
          return { id: article.dataset.messageId ?? "", offset: box.top - origin };
        }
      }
      return { id: "", offset: 0 };
    });
    await scroller.evaluate((element, id) => {
      const articles = Array.from(element.querySelectorAll<HTMLElement>("[data-message-id]"));
      const above = articles[articles.findIndex((a) => a.dataset.messageId === id) - 1];
      const picture = document.createElement("div");
      picture.style.height = "400px";
      above?.append(picture);
    }, reading.id);
    // Within a pixel: WebKit scrolls in whole pixels.
    await expect
      .poll(async () =>
        Math.abs(
          (await scroller.evaluate((element, id) => {
            const article = element.querySelector(`[data-message-id="${id}"]`);
            return article === null
              ? Infinity
              : article.getBoundingClientRect().top - element.getBoundingClientRect().top;
          }, reading.id)) - reading.offset,
        ),
      )
      .toBeLessThanOrEqual(1);
  });

  test("tapping a message's picture shows who wrote it", async ({ page }) => {
    await openChannel(page, "general");
    const own = messageWith(page, ownText);
    // The name and the picture both open the card; the picture comes first.
    await own.getByRole("button", { name: "Show profile of Kate" }).first().tap();
    await expect(page.getByRole("dialog", { name: /Kate/ })).toBeVisible();
  });

  test("an invite shows its code, and copies its link without the Clipboard API", async ({
    page,
  }) => {
    // A page served over plain HTTP, like a dev server a phone reaches by address, has none.
    await page.evaluate(() => {
      Reflect.deleteProperty(Navigator.prototype, "clipboard");
    });
    await page.getByRole("button", { name: "Invite people" }).tap();
    const invite = page.getByRole("dialog").locator("li").first();
    await expect(invite.locator("code")).toHaveText(inviteCode);
    await invite.getByRole("button", { name: "Copy link" }).tap();
    await expect(invite.getByRole("button", { name: "Copied" })).toBeVisible();
  });

  test("icon controls answer a fingertip", async ({ page }) => {
    await expectTouchable(page.getByRole("button", { name: "Settings", exact: true }));
    await expectTouchable(page.getByRole("button", { name: "Edit profile" }));
    await expectTouchable(page.getByRole("button", { name: "Add a channel to Planning" }));
    // The sidebar header's icon buttons sit side by side, drawn big enough on their own.
    await expectTouchable(page.getByRole("button", { name: "Community settings" }));
    await expectTouchable(page.getByRole("button", { name: "Invite people" }));
    // Always shown on a touch screen, where there is no right click or hover.
    await expectTouchable(page.getByRole("button", { name: "Options for roadmap" }));
    await openChannel(page, "general");
    await expectTouchable(page.getByRole("link", { name: "Back to channels" }));
    await page.getByRole("link", { name: /2 replies/ }).click();
    await expectTouchable(page.getByRole("button", { name: "Close thread" }));
    await page.getByRole("button", { name: "Close thread" }).click();
    await page.getByRole("link", { name: "Back to channels" }).click();
    await rail(page).getByRole("link", { name: "Direct messages" }).click();
    await expectTouchable(page.getByRole("button", { name: "New message" }));
  });

  test("a poll's own answer field and its remove control fit a phone", async ({ page }) => {
    await openChannel(page, "general");
    const poll = page.getByRole("region", { name: `Poll: ${pollQuestion}` });
    await poll.getByRole("textbox", { name: "Add your own answer" }).fill("Tacos");
    await poll.getByRole("button", { name: "Add", exact: true }).tap();
    await expectTouchable(poll.getByRole("button", { name: "Remove the answer Tacos" }));
    await expectNoSidewaysScroll(page);
  });

  test("a dialog taller than the screen fits on it and scrolls to its end", async ({
    page,
    browserName,
  }) => {
    await openChannel(page, "general");
    await page.getByRole("button", { name: "Create a new poll" }).tap();
    const form = page.getByRole("dialog", { name: "New poll" });
    await expect(form).toBeVisible();
    // Enough options to be taller than any phone.
    for (let i = 0; i < 6; i++) {
      await form.getByRole("button", { name: "Add option" }).tap();
    }
    const frame = form.locator("xpath=..");
    const fit = await frame.evaluate((element) => {
      const box = element.getBoundingClientRect();
      return {
        top: box.top,
        bottom: box.bottom,
        screen: window.innerHeight,
        overflows: element.scrollHeight > element.clientHeight,
      };
    });
    expect(fit.top).toBeGreaterThanOrEqual(0);
    expect(fit.bottom).toBeLessThanOrEqual(fit.screen);
    expect(fit.overflows).toBe(true);
    // A swipe up the dialog reaches the end of the form. Only Chromium takes synthetic touch
    // input; elsewhere the dialog is scrolled directly.
    const box = await frame.boundingBox();
    if (box === null) {
      throw new Error("the dialog has no box");
    }
    if (browserName === "chromium") {
      for (let i = 0; i < 8; i++) {
        await swipe(page, box.x + box.width / 2, box.y + box.height - 40, box.y + 40);
      }
    } else {
      await frame.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
    }
    const create = form.getByRole("button", { name: "Create a poll" });
    const button = await create.boundingBox();
    expect(button).not.toBeNull();
    expect((button?.y ?? Infinity) + (button?.height ?? 0)).toBeLessThanOrEqual(fit.bottom);
  });

  /** Opens the reaction picker on the caller's own message and measures how it sits. */
  async function openReactionPicker(page: Page) {
    await openChannel(page, "general");
    const own = messageWith(page, ownText);
    await own.locator(".message-body").tap();
    await own.getByRole("button", { name: "Add a reaction" }).tap();
    const picker = page.locator(".emoji-picker");
    await expect(picker).toBeVisible();
    // Measured the moment its first emoji show: the library places them before it has measured
    // one, and they must fit then too.
    await expect(picker.locator(".epr-emoji-category-content > *").first()).toBeVisible();
    return picker.evaluate((element) => {
      const box = element.getBoundingClientRect();
      const list = element.querySelector<HTMLElement>(".epr-body");
      const firstRow = Array.from(
        element.querySelectorAll<HTMLElement>(".epr-emoji-category-content > *"),
      )
        .map((emoji) => emoji.getBoundingClientRect())
        .filter((rect, _, all) => rect.width > 0 && rect.top === all[0]?.top);
      const inner = list?.getBoundingClientRect();
      return {
        left: box.left,
        right: box.right,
        screen: window.innerWidth,
        listOverflow: list === null ? 0 : list.scrollWidth - list.clientWidth,
        perRow: firstRow.length,
        before:
          inner === undefined || firstRow[0] === undefined ? 0 : firstRow[0].left - inner.left,
        after:
          inner === undefined || list === null || firstRow.length === 0
            ? 0
            : inner.left + list.clientWidth - (firstRow.at(-1)?.right ?? 0),
      };
    });
  }

  test("the reaction picker fits on the screen", async ({ page }) => {
    const fit = await openReactionPicker(page);
    expect(fit.left).toBeGreaterThanOrEqual(0);
    expect(fit.right).toBeLessThanOrEqual(fit.screen);
    expect(fit.listOverflow).toBeLessThanOrEqual(0);
    // A whole row of emoji, as far from each side.
    expect(fit.perRow).toBe(8);
    expect(Math.abs(fit.before - fit.after)).toBeLessThanOrEqual(1);
  });

  test("on the narrowest phones the reaction picker takes a row of seven", async ({ page }) => {
    await page.setViewportSize({ width: 340, height: 700 });
    const fit = await openReactionPicker(page);
    expect(fit.left).toBeGreaterThanOrEqual(0);
    expect(fit.right).toBeLessThanOrEqual(fit.screen);
    expect(fit.listOverflow).toBeLessThanOrEqual(0);
    expect(fit.perRow).toBe(7);
    expect(Math.abs(fit.before - fit.after)).toBeLessThanOrEqual(1);
  });
});
