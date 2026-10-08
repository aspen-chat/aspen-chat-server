import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  dm,
  inviteCode,
  longPress,
  ownText,
  pollQuestion,
  reactedText,
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

/**
 * Drags a finger across the screen at height `y` from `fromX` to `toX` (Chromium only). With
 * `rest`, it comes to rest before it lifts, rather than being thrown.
 */
async function swipeAcross(
  page: Page,
  y: number,
  fromX: number,
  toX: number,
  { rest = false } = {},
) {
  const client = await page.context().newCDPSession(page);
  const steps = 8;
  await client.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: fromX, y }],
  });
  for (let i = 1; i <= steps; i++) {
    await client.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x: fromX + ((toX - fromX) * i) / steps, y }],
    });
  }
  if (rest) {
    await page.waitForTimeout(200);
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

  test("a swipe toward the start draws out the members, and a swipe back puts them away", async ({
    page,
    browserName,
  }) => {
    test.skip(browserName !== "chromium", "only Chromium takes synthetic touch input");
    await openChannel(page, "general");
    await expect(messageWith(page, starterText)).toBeVisible();
    const width = page.viewportSize()?.width ?? 0;
    const height = page.viewportSize()?.height ?? 0;
    const members = page.getByRole("dialog", { name: "Members" });
    await swipeAcross(page, height / 2, width - 40, 40);
    await expect(members).toBeVisible();
    await expect(
      members.getByRole("button", { name: /Bob With A Rather Long Display Name/ }),
    ).toBeVisible();
    await settleAnimations(page);
    const box = await members.boundingBox();
    expect(box?.x ?? 0).toBeGreaterThan(0);
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeCloseTo(width, 0);
    await expectNoSidewaysScroll(page);
    await swipeAcross(page, height / 2, (box?.x ?? 0) + 20, width - 10);
    await expect(members).toBeHidden();
    await expect(page.getByRole("heading", { name: "general" })).toBeVisible();
  });

  test("a swipe that comes to rest short of halfway springs back, and one up the list opens nothing", async ({
    page,
    browserName,
  }) => {
    test.skip(browserName !== "chromium", "only Chromium takes synthetic touch input");
    await openChannel(page, "general");
    await expect(messageWith(page, starterText)).toBeVisible();
    const width = page.viewportSize()?.width ?? 0;
    const height = page.viewportSize()?.height ?? 0;
    const members = page.getByRole("dialog", { name: "Members" });
    await swipeAcross(page, height / 2, width - 40, width - 120, { rest: true });
    await expect(members).toBeHidden();
    await swipe(page, width / 2, height / 2 + 100, height / 2 - 100);
    await expect(members).toBeHidden();
  });

  test("the channel's members button opens the members, and their X puts them away", async ({
    page,
  }) => {
    await openChannel(page, "general");
    await page.getByRole("button", { name: "Show members" }).click();
    const members = page.getByRole("dialog", { name: "Members" });
    await expect(members).toBeVisible();
    await expect(members.getByRole("heading", { name: /Online/ })).toBeVisible();
    await expectTouchable(members.getByRole("button", { name: "Close" }));
    await members.getByRole("button", { name: "Close" }).click();
    await expect(members).toBeHidden();
    await expect(page.getByRole("button", { name: "Show members" })).toBeFocused();
  });

  test("a long press opens a message's actions in a sheet from the bottom, and each action closes it", async ({
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
    const sheet = page.getByRole("dialog", { name: "Message actions" });
    const quick = sheet.getByRole("group", { name: "Quick reactions" });
    const list = sheet.getByRole("group", { name: "Message actions" });
    // A tap focuses the message, as a tap on a link needs, and offers nothing.
    await own.locator(".message-body").tap();
    await expect(sheet).toHaveCount(0);
    await expect(actionsOf(own)).toHaveCount(0);
    // A long press opens the actions in a sheet across the bottom of the screen.
    await longPress(page, own.locator(".message-body"));
    await expect(sheet).toBeVisible();
    await settleAnimations(page);
    const viewport = page.viewportSize();
    const box = await sheet.boundingBox();
    expect(box?.width ?? 0).toBeCloseTo(viewport?.width ?? 0, 0);
    expect((box?.y ?? 0) + (box?.height ?? 0)).toBeCloseTo(viewport?.height ?? 0, 0);
    // Above the list, the reader's most used emoji, then the defaults, then the picker's
    // button, unnamed on screen; it is not in the list again.
    const quickNames = ["🎉", "👍", "😄", "❤️", "👎"].map((emoji) => `React with ${emoji}`);
    await expect(quick.getByRole("button")).toHaveCount(6);
    for (const [i, name] of [...quickNames, "Add a reaction"].entries()) {
      const button = quick.getByRole("button").nth(i);
      await expect(button).toHaveAccessibleName(name);
      await expectTouchable(button);
    }
    await expect(quick.getByRole("button", { name: "Add a reaction" })).toHaveText("");
    await expect(list.getByRole("button", { name: "Add a reaction" })).toHaveCount(0);
    // The list names each action beside its icon, each a full row a finger can hit.
    for (const name of ["Edit message", "Delete message", "Copy text"]) {
      const action = list.getByRole("button", { name });
      await expect(action).toHaveText(name);
      await expectTouchable(action);
    }
    // Copying the text closes the actions and says so in a toast.
    await list.getByRole("button", { name: "Copy text" }).tap();
    await expect(sheet).toHaveCount(0);
    await expect(page.getByText("Copied text")).toBeVisible();
    // Adding a reaction closes the actions and opens the picker in their place, which stays.
    await longPress(page, own.locator(".message-body"));
    await quick.getByRole("button", { name: "Add a reaction" }).tap();
    await expect(sheet).toHaveCount(0);
    const picker = page.locator(".emoji-picker");
    await expect(picker).toBeVisible();
    await page.waitForTimeout(500);
    await expect(picker).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(picker).toHaveCount(0);
    // A tap on the darkened screen above the sheet closes it; a long press on another message
    // opens that one's.
    await longPress(page, own.locator(".message-body"));
    await expect(sheet).toBeVisible();
    await settleAnimations(page);
    await page.touchscreen.tap((viewport?.width ?? 0) / 2, 20);
    await expect(sheet).toHaveCount(0);
    await longPress(page, starter.locator(".message-body"));
    await expect(sheet).toBeVisible();
    await expect(list.getByRole("button", { name: "Reply in thread" })).toBeVisible();
    // The actions take focus once the press ends, so a screen reader is in them; Escape then
    // closes them.
    await expect(sheet).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(sheet).toHaveCount(0);
  });

  test("a quick reaction reacts and closes the sheet, and one already chosen takes it back", async ({
    page,
  }) => {
    await openChannel(page, "general");
    const reacted = messageWith(page, reactedText);
    const sheet = page.getByRole("dialog", { name: "Message actions" });
    const quick = sheet.getByRole("group", { name: "Quick reactions" });
    const chips = reacted.getByRole("list", { name: "Reactions" });
    // The caller already reacted 👍 here, so its quick reaction is pressed, and takes it back.
    await longPress(page, reacted.locator(".message-body"));
    await expect(quick.getByRole("button", { name: "Remove your 👍" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await quick.getByRole("button", { name: "Remove your 👍" }).tap();
    await expect(sheet).toHaveCount(0);
    await expect(chips.getByRole("button", { name: "React with 👍" })).toBeVisible();
    // 🎉 is not theirs yet: one tap adds it.
    await longPress(page, reacted.locator(".message-body"));
    await quick.getByRole("button", { name: "React with 🎉" }).tap();
    await expect(sheet).toHaveCount(0);
    await expect(chips.getByRole("button", { name: "Remove your 🎉" })).toBeVisible();
  });

  test("a swipe down on the actions' sheet puts it away", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "only Chromium takes synthetic touch input");
    await openChannel(page, "general");
    const own = messageWith(page, ownText);
    const sheet = page.getByRole("dialog", { name: "Message actions" });
    await longPress(page, own.locator(".message-body"));
    await expect(sheet).toBeVisible();
    await settleAnimations(page);
    const box = await sheet.boundingBox();
    const x = (box?.x ?? 0) + (box?.width ?? 0) / 2;
    const top = (box?.y ?? 0) + 6;
    await swipe(page, x, top, top + (box?.height ?? 0));
    await expect(sheet).toHaveCount(0);
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
      // The first row of the fullest category: the community's own section comes first and
      // may hold a single emoji, which says nothing about how many fit across.
      const categories = Array.from(
        element.querySelectorAll<HTMLElement>(".epr-emoji-category-content"),
      ).sort((a, b) => b.childElementCount - a.childElementCount);
      const firstRow = Array.from(categories[0]?.children ?? [])
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

  for (const ios of [false, true]) {
    test(`a finger in the reaction picker's sheet leaves the list behind it still${ios ? ", with the list scrolling itself as on iOS" : ""}`, async ({
      page,
      browserName,
    }) => {
      test.skip(browserName !== "chromium", "only Chromium takes synthetic touch input");
      if (ios) {
        // Claims to be iOS, so the list scrolls itself from its own touch handlers.
        await page.addInitScript(() => {
          const supports = CSS.supports.bind(CSS) as (property: string, value: string) => boolean;
          CSS.supports = ((property: string, value: string) =>
            property === "-webkit-touch-callout" ||
            supports(property, value)) as typeof CSS.supports;
        });
        // Signed in already, by the page before the claim: loaded again, the page makes it.
        await page.reload();
        await expect(channelList(page)).toBeVisible();
      }
      await openReactionPicker(page);
      await settleAnimations(page);
      // Where a message behind the sheet stands on the screen: the list's own position moves
      // as history pages in above, with nothing on screen moving.
      const behind = messageWith(page, ownText);
      const before = (await behind.boundingBox())?.y;
      // Partway down the emoji, so a finger moving down the sheet scrolls the picker back
      // rather than pulling the sheet away; a list taking the finger too would draw older
      // messages into view.
      const emoji = page.locator(".emoji-picker .epr-body");
      await emoji.evaluate((element) => {
        element.scrollTop = 400;
      });
      const box = await emoji.boundingBox();
      const x = (box?.x ?? 0) + 100;
      const y = (box?.y ?? 0) + 20;
      await swipe(page, x, y, y + 150);
      await page.waitForTimeout(500);
      expect(await emoji.evaluate((element) => element.scrollTop)).toBeLessThan(400);
      expect((await behind.boundingBox())?.y).toBe(before);
    });
  }

  for (const width of [412, 340]) {
    test(`the reaction picker fills a sheet across a ${String(width)}px phone in whole rows`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 800 });
      const fit = await openReactionPicker(page);
      const sheet = page.getByRole("dialog", { name: "Add a reaction" });
      await settleAnimations(page);
      const box = await sheet.boundingBox();
      expect((box?.y ?? 0) + (box?.height ?? 0)).toBeCloseTo(800, 0);
      expect(box?.width ?? 0).toBeCloseTo(width, 0);
      expect(fit.left).toBeGreaterThanOrEqual(0);
      expect(fit.right).toBeLessThanOrEqual(fit.screen);
      expect(fit.listOverflow).toBeLessThanOrEqual(0);
      // As many whole 40px emoji as the screen holds beside the picker's padding, as far from
      // each side.
      expect(fit.perRow).toBe(Math.floor((width - 20) / 40));
      expect(Math.abs(fit.before - fit.after)).toBeLessThanOrEqual(1);
      // Its search waits for a tap, so the keyboard does not rise over the emoji.
      await expect(sheet.getByRole("searchbox").or(sheet.locator("input"))).not.toBeFocused();
    });
  }

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
