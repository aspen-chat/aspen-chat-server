import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  dm,
  inviteCode,
  longPress,
  ownText,
  pollQuestion,
  settleAnimations,
  signInToWorld,
  starterText,
} from "./world";

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

/**
 * Scrolls the message list by `deltaY` pixels, as a script would: its box is scrolled by the
 * browser here and by the list itself on iOS, and takes a script's scroll either way.
 */
async function scrollListBy(list: Locator, deltaY: number) {
  await list.evaluate((element, by) => {
    element.scrollTop += by;
  }, deltaY);
}

/** Scrolls the message list to `offset` pixels from the top of what it holds. */
async function scrollListTo(list: Locator, offset: number) {
  await list.evaluate((element, to) => {
    element.scrollTop = to;
  }, offset);
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

  test("a long press opens a message's actions beside the finger, and each action closes them", async ({
    page,
    context,
    browserName,
  }) => {
    // Chromium asks leave to write the clipboard; the others have no such permission to give.
    if (browserName === "chromium") {
      await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    }
    await openChannel(page, "general");
    const own = messageWith(page, ownText);
    const starter = messageWith(page, starterText);
    const popover = page.getByRole("dialog", { name: "Message actions" });
    const composer = page.getByRole("textbox", { name: "Message" });
    // A tap focuses the message, as a tap on a link needs, and offers nothing.
    await own.locator(".message-body").tap();
    await expect(popover).toHaveCount(0);
    await expect(actionsOf(own)).toHaveCount(0);
    // A long press opens the actions in a popover beside the finger, each at finger size.
    const press = await longPress(page, own.locator(".message-body"));
    await expect(popover).toBeVisible();
    const near = async () => {
      const box = await popover.boundingBox();
      const composerBox = await composer.boundingBox();
      expect(box).not.toBeNull();
      const top = box?.y ?? 0;
      const bottom = top + (box?.height ?? 0);
      expect(Math.min(Math.abs(top - press.y), Math.abs(bottom - press.y))).toBeLessThan(100);
      // Never over the message box.
      expect(bottom).toBeLessThanOrEqual((composerBox?.y ?? 0) + 1);
      return { top, bottom };
    };
    await near();
    for (const name of ["Add a reaction", "Edit message", "Delete message", "Copy text"]) {
      const action = popover.getByRole("button", { name });
      await expect(action).toBeVisible();
      expect((await action.boundingBox())?.height).toBeGreaterThanOrEqual(40);
    }
    // Copying the text closes the actions and says so in a toast.
    await popover.getByRole("button", { name: "Copy text" }).tap();
    await expect(popover).toHaveCount(0);
    await expect(page.getByText("Copied text")).toBeVisible();
    // Adding a reaction closes the actions and opens the picker in their place, which stays.
    await longPress(page, own.locator(".message-body"));
    await popover.getByRole("button", { name: "Add a reaction" }).tap();
    await expect(popover).toHaveCount(0);
    const picker = page.locator(".emoji-picker");
    await expect(picker).toBeVisible();
    await page.waitForTimeout(500);
    await expect(picker).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(picker).toHaveCount(0);
    // A tap elsewhere closes them (the tap lands on the popover's underlay, so it is sent by
    // position, to a corner of the list the popover is not over); a long press on another
    // message opens that one's.
    await longPress(page, own.locator(".message-body"));
    await expect(popover).toBeVisible();
    await settleAnimations(page);
    const list = await page.locator("[data-message-list]").boundingBox();
    const open = await popover.boundingBox();
    const listTop = (list?.y ?? 0) + 6;
    const listBottom = (list?.y ?? 0) + (list?.height ?? 0) - 6;
    const overTop = open !== null && open.y < listTop + 6;
    await page.touchscreen.tap((list?.x ?? 0) + 6, overTop ? listBottom : listTop);
    await expect(popover).toHaveCount(0);
    await longPress(page, starter.locator(".message-body"));
    await expect(popover).toBeVisible();
    await expect(popover.getByRole("button", { name: "Reply in thread" })).toBeVisible();
    // The actions take focus once the press ends, so a screen reader is in them; Escape then
    // closes them.
    await expect(popover).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(popover).toHaveCount(0);
    // On the newest message, which sits on the message box, they open above the finger.
    const newest = page.locator("article[data-message-id]").last();
    const pressed = await longPress(page, newest.locator(".message-body"));
    await expect(popover).toBeVisible();
    const box = await popover.boundingBox();
    expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(pressed.y);
  });

  test("a link in a message opens on the first tap", async ({ page }) => {
    // Tapping a link focuses its message first; nothing that follows may move the link out
    // from under the finger before the tap completes.
    await openChannel(page, "general");
    await page.getByRole("link", { name: /2 replies/ }).tap();
    await expect(page.getByRole("heading", { name: "Thread" })).toBeVisible();
  });

  test("reading back through history keeps the message in view still", async ({ page }) => {
    await openChannel(page, "general");
    const scroller = page.locator("[data-message-list]");
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
    await scrollListTo(scroller, 400);
    await expect(scroller.getByText("Loading…")).toBeVisible();
    // The reader keeps going while the page is on its way.
    await scrollListBy(scroller, -150);
    const reading = await topmost();
    await expect.poll(() => articles.count()).toBeGreaterThan(before);
    await expect(scroller.getByText("Loading…")).toBeHidden();
    // Within a pixel: WebKit scrolls in whole pixels.
    expect(Math.abs((await offsetOf(reading.id)) - reading.offset)).toBeLessThanOrEqual(1);
  });

  test("content growing above what is being read does not move it", async ({ page }) => {
    // As a picture does when it loads above the reader: the list keeps the message being
    // read where it was.
    await openChannel(page, "general");
    const scroller = page.locator("[data-message-list]");
    await expect(scroller.locator("article").first()).toBeVisible();
    await scrollListBy(scroller, -300);
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
    // On a phone the poll is one of the box's other controls, behind its + button.
    await page.getByRole("button", { name: "Add a file or a poll" }).tap();
    await page.getByRole("menuitem", { name: "Create a new poll" }).tap();
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
    await longPress(page, own.locator(".message-body"));
    await page
      .getByRole("dialog", { name: "Message actions" })
      .getByRole("button", { name: "Add a reaction" })
      .tap();
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

  test("the message box sits level with its buttons, which share one + menu", async ({ page }) => {
    await openChannel(page, "general");
    const box = page.getByRole("textbox", { name: "Message" });
    const more = page.getByRole("button", { name: "Add a file or a poll" });
    const send = page.getByRole("button", { name: "Send" });
    const tops = await Promise.all([box, more, send].map((l) => l.boundingBox()));
    expect(new Set(tops.map((b) => Math.round(b?.y ?? -1))).size).toBe(1);
    expect(new Set(tops.map((b) => Math.round(b?.height ?? -1))).size).toBe(1);
    await more.tap();
    await expect(page.getByRole("menuitem")).toHaveText(["Attach a file", "Create a new poll"]);
  });

  test("Enter writes a new line, and only the button sends", async ({ page }) => {
    await openChannel(page, "general");
    const box = page.getByRole("textbox", { name: "Message" });
    let posted = false;
    page.on("request", (request) => {
      if (request.method() === "POST" && request.url().includes("/messages")) {
        posted = true;
      }
    });
    await box.fill("first line");
    await box.press("Enter");
    await box.pressSequentially("second");
    await expect(box).toHaveValue("first line\nsecond");
    expect(posted).toBe(false);
  });

  test("a placeholder too long for the box is cut short, never making it taller", async ({
    page,
  }) => {
    await page.goto(`/dms/${dm}`);
    const box = page.getByRole("textbox", { name: "Message" });
    // A whole page load, the DM's people included, which takes longer on a busy machine.
    await expect(box).toHaveAttribute("aria-placeholder", /Bob With A Rather Long Display Name/, {
      timeout: 15_000,
    });
    const more = page.getByRole("button", { name: "Add a file or a poll" });
    await expect
      .poll(async () => {
        const [boxSize, moreSize] = await Promise.all([box.boundingBox(), more.boundingBox()]);
        return Math.round(boxSize?.height ?? 0) - Math.round(moreSize?.height ?? -1);
      })
      .toBe(0);
    // The placeholder is drawn just after the box, on one line, ending in an ellipsis.
    await expect
      .poll(() =>
        page.locator("textarea + span").evaluate((span) => span.scrollWidth > span.clientWidth),
      )
      .toBe(true);
  });

  test("the view keeps its bottom when the keyboard takes the screen's space", async ({ page }) => {
    await openChannel(page, "general");
    const list = page.locator("[data-message-list]");
    await expect(list.locator("article").first()).toBeVisible();
    await scrollListBy(list, -200);
    const lowest = () =>
      list.evaluate((element) => {
        const view = element.getBoundingClientRect();
        let found: [string, number] | null = null;
        for (const article of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
          const rect = article.getBoundingClientRect();
          if (rect.top < view.bottom - 4 && rect.bottom > view.top) {
            found = [article.dataset.messageId ?? "", Math.round(view.bottom - rect.bottom)];
          }
        }
        return found;
      });
    await page.waitForTimeout(300);
    const before = await lowest();
    const size = page.viewportSize() ?? { width: 390, height: 844 };
    await page.setViewportSize({ width: size.width, height: size.height - 300 });
    await expect.poll(lowest).toEqual(before);
  });
});
