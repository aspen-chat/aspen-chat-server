import { expect, test, type CDPSession, type Page } from "@playwright/test";
import { bob, community, general, me, signInToWorld } from "./world";

/**
 * Reading back through a long history on a phone, against the stubbed world, with #general
 * given 600 messages: tall pictures (most with their size known, some without, as older
 * uploads are), long paragraphs, and short lines, history pages taking a normal connection's
 * time, and pictures taking their own. A finger drags the list down slowly, a few pixels at a
 * time, and two things must hold the whole way:
 *
 * 1. The reader never waits: the next page is read far enough ahead that the top of what is
 *    loaded, where a page would be awaited, never comes into view.
 * 2. Nothing moves by itself: each step of the finger moves every message in view by exactly
 *    that step, and while the finger rests, nothing moves at all.
 */

const COUNT = 600;
/** How long a page of history takes to come, as on an ordinary phone connection. */
const PAGE_DELAY_MS = 300;
/** How far the finger moves at each step, and how many steps it takes. */
const STEP_PX = 12;
const STEPS = 2000;
/** How many flicks the flicking test makes. */
const FLICKS = 40;
/** The share of frames in which the flicking test lets the top of what is loaded show. */
const SEAM_SHARE = 0.02;

const id = (n: number) => `0190f0a0-0000-7000-8002-${n.toString(16).padStart(12, "0")}`;
const attachmentId = (n: number) => `0190f0a0-0000-7000-8003-${n.toString(16).padStart(12, "0")}`;

/** A deterministic spread of numbers, so every run reads the same history. */
function spread(n: number, max: number): number {
  return ((n * 2654435761) >>> 0) % max;
}

const PARAGRAPH =
  "We walked the long way round the reservoir, past the boathouse and the old pumping station, " +
  "and stopped where the path climbs into the beeches to watch the light go. ";

interface Picture {
  id: string;
  width: number;
  height: number;
  known: boolean;
}

/** Message `n` (1 the oldest): a tall picture every third, a paragraph or a line otherwise. */
function messageOf(n: number): { record: Record<string, unknown>; picture: Picture | null } {
  const picture: Picture | null =
    n % 3 === 0
      ? {
          id: attachmentId(n),
          // Most are tall, as phone photos are; every seventh is wide, wider than the column.
          width: n % 7 === 0 ? 1600 : 600,
          height: n % 7 === 0 ? 900 : 900 + spread(n, 600),
          // Every fifth picture is from before sizes were recorded.
          known: n % 5 !== 0,
        }
      : null;
  const content =
    picture !== null ? "" : n % 3 === 1 ? PARAGRAPH.repeat(1 + spread(n, 4)) : `Line ${String(n)}`;
  return {
    record: {
      id: id(n),
      channelId: general,
      author: n % 2 === 0 ? me : bob,
      timestamp: new Date(Date.UTC(2026, 0, 1) + n * 60_000).toISOString(),
      editedAt: null,
      linkPreviews: [],
      linkedMessages: [],
      kind: "standard",
      poll: null,
      thread: null,
      echoOf: null,
      content,
      attachments: picture === null ? [] : [picture.id],
      mentions: { users: [], roles: [], everyone: false },
    },
    picture,
  };
}

const history = Array.from({ length: COUNT }, (_, i) => messageOf(COUNT - i)); // newest first

function attachmentOf(picture: Picture) {
  return {
    id: picture.id,
    fileName: `${picture.id}.svg`,
    mimeType: "image/svg+xml",
    downloadUrl: `https://media.test/${picture.id}.svg`,
    width: picture.known ? picture.width : null,
    height: picture.known ? picture.height : null,
  };
}

/** #general's history, paged as the server pages it, newest first. */
async function serveHistory(page: Page, messages: typeof history = history) {
  await page.route(`**/api/v1/channels/${general}/messages*`, async (route) => {
    const url = new URL(route.request().url());
    const limit = Number(url.searchParams.get("limit") ?? "50");
    const before = url.searchParams.get("before");
    const after = url.searchParams.get("after");
    const around = url.searchParams.get("around");
    let slice: typeof history;
    if (around !== null) {
      // The message and up to `limit` on each side of it, newest first like the rest.
      await new Promise((resolve) => setTimeout(resolve, PAGE_DELAY_MS));
      const at = messages.findIndex((m) => m.record.id === around);
      slice = at === -1 ? [] : messages.slice(Math.max(0, at - limit), at + limit + 1);
    } else if (before !== null) {
      await new Promise((resolve) => setTimeout(resolve, PAGE_DELAY_MS));
      const start = messages.findIndex((m) => (m.record.id as string) < before);
      slice = start === -1 ? [] : messages.slice(start, start + limit);
    } else if (after !== null) {
      const newer = messages.filter((m) => (m.record.id as string) > after).reverse();
      slice = newer.slice(0, limit).reverse();
    } else {
      slice = messages.slice(0, limit);
    }
    await route.fulfill({
      json: {
        data: slice.map((m) => m.record),
        included: {
          users: [],
          attachments: slice.flatMap((m) => (m.picture === null ? [] : [attachmentOf(m.picture)])),
        },
      },
    });
  });
  // Each picture arrives in its own time, as pictures do.
  await page.route("https://media.test/**", async (route) => {
    const name = new URL(route.request().url()).pathname.slice(1).replace(".svg", "");
    const picture = history.find((m) => m.picture?.id === name)?.picture;
    if (picture === undefined || picture === null) {
      await route.fulfill({ status: 404 });
      return;
    }
    // Most take a moment; every fourth takes half a second or more, as on a slow connection.
    const slow = spread(picture.height, 4) === 0;
    const delay = slow ? 500 + spread(picture.height, 1000) : 150 + spread(picture.height, 500);
    await new Promise((resolve) => setTimeout(resolve, delay));
    await route.fulfill({
      contentType: "image/svg+xml",
      body:
        `<svg xmlns="http://www.w3.org/2000/svg" width="${String(picture.width)}" height="${String(picture.height)}">` +
        `<rect width="100%" height="100%" fill="#${(spread(picture.height, 0xffffff) | 0x404040).toString(16)}"/></svg>`,
    });
  });
}

/**
 * What the list shows: where each message in view sits, and whether the top of what is loaded
 * shows. Read after a whole rendering update, so that what the list does before a frame is
 * painted (its resize observers run between layout and paint) is in.
 */
function sample(page: Page) {
  return page.evaluate(async () => {
    await new Promise((resolve) => {
      requestAnimationFrame(() => {
        setTimeout(resolve, 0);
      });
    });
    const scroller = document.querySelector("article")?.closest("[data-message-list]");
    if (!(scroller instanceof HTMLElement)) {
      return null;
    }
    const view = scroller.getBoundingClientRect();
    const tops: Record<string, number> = {};
    const heights: Record<string, number> = {};
    for (const article of scroller.querySelectorAll<HTMLElement>("article[data-message-id]")) {
      const rect = article.getBoundingClientRect();
      heights[article.dataset.messageId ?? ""] = rect.height;
      if (rect.bottom > view.top && rect.top < view.bottom) {
        tops[article.dataset.messageId ?? ""] = rect.top - view.top;
      }
    }
    // The space at the top of the list a page of history is awaited in.
    const seam = scroller.querySelector('p[aria-live="polite"]');
    const seamRect = seam?.getBoundingClientRect();
    const seamShows =
      seamRect !== undefined && seamRect.bottom > view.top && seamRect.top < view.bottom;
    return {
      tops,
      heights,
      seamShows,
      count: Object.keys(heights).length,
      first: scroller.querySelector("article")?.getAttribute("data-message-id") ?? "",
      scrollTop: scroller.scrollTop,
    };
  });
}

/** Two frames, by which a touch's scroll has been drawn. */
function frames(page: Page) {
  return page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            resolve();
          });
        });
      }),
  );
}

/** How far the messages in view in both samples moved, each; empty when none is in both. */
function moves(before: Record<string, number>, after: Record<string, number>): number[] {
  return Object.keys(before)
    .filter((key) => key in after)
    .map((key) => (after[key] ?? 0) - (before[key] ?? 0));
}

/**
 * The two ways the list's box is scrolled: by the browser, as everywhere but iOS, and by the
 * list itself, as on iOS, which Chromium is made to stand in for by claiming the property
 * the list knows iOS by (`OWNS_SCROLLING`). Both must keep the view still the same.
 */
const SCROLLERS = [
  { name: "", ios: false },
  { name: ", with the list scrolling itself as on iOS", ios: true },
];

/** Makes the page claim to be iOS, so the list scrolls itself. */
async function asIos(page: Page) {
  await page.addInitScript(() => {
    const supports = CSS.supports.bind(CSS) as (property: string, value: string) => boolean;
    CSS.supports = ((property: string, value: string) =>
      property === "-webkit-touch-callout" || supports(property, value)) as typeof CSS.supports;
  });
}

async function touch(
  cdp: CDPSession,
  type: "touchStart" | "touchMove" | "touchEnd",
  x: number,
  y: number,
) {
  await cdp.send("Input.dispatchTouchEvent", {
    type,
    touchPoints: type === "touchEnd" ? [] : [{ x, y }],
  });
}

for (const { name, ios } of SCROLLERS) {
  test(`reading slowly back through a long history never waits and never moves by itself${name}`, async ({
    page,
    browserName,
    isMobile,
  }) => {
    test.skip(
      !isMobile || browserName !== "chromium",
      "a finger is driven through Chromium's protocol",
    );
    test.setTimeout(600_000);
    if (ios) {
      await asIos(page);
    }
    await signInToWorld(page, serveHistory);
    await page
      .getByRole("grid", { name: "Channels" })
      .first()
      .getByText("general", { exact: true })
      .click();
    await expect(page.locator(`article[data-message-id="${id(COUNT)}"]`)).toBeVisible();
    // The pictures already in view have settled.
    await page.waitForTimeout(1500);

    const cdp = await page.context().newCDPSession(page);
    const box = await page
      .locator("article")
      .first()
      .evaluate((article) => {
        const rect = article.closest("[data-message-list]")?.getBoundingClientRect();
        return rect === undefined
          ? null
          : { x: rect.x + rect.width / 2, top: rect.top, bottom: rect.bottom };
      });
    expect(box).not.toBeNull();
    const x = box?.x ?? 200;
    const top = (box?.top ?? 100) + 40;
    const bottom = (box?.bottom ?? 700) - 40;

    const strays: string[] = [];
    const seams: number[] = [];
    let reached: Awaited<ReturnType<typeof sample>> = null;
    let finger = top;
    await touch(cdp, "touchStart", x, finger);
    // Past the slop a touch moves before it scrolls.
    finger += 20;
    await touch(cdp, "touchMove", x, finger);
    await frames(page);
    let last = await sample(page);
    for (let step = 0; step < STEPS; step++) {
      if (finger + STEP_PX > bottom) {
        // The finger rests, lifts, and comes back to the top, as a reader's does; the list must
        // stay where it was left while it is away. A finger lifted while moving flings instead.
        await page.waitForTimeout(120);
        await touch(cdp, "touchEnd", x, finger);
        const resting = await sample(page);
        await page.waitForTimeout(400);
        const rested = await sample(page);
        if (resting !== null && rested !== null) {
          const both = moves(resting.tops, rested.tops);
          const drift = both.filter((d) => Math.abs(d) > 1);
          if (both.length === 0) {
            strays.push(`resting before step ${String(step)}: nothing in view stayed in view`);
          } else if (drift.length > 0) {
            strays.push(`resting before step ${String(step)}: moved ${drift.join(", ")}px`);
          }
        }
        finger = top;
        await touch(cdp, "touchStart", x, finger);
        finger += 20;
        await touch(cdp, "touchMove", x, finger);
        await frames(page);
        last = await sample(page);
        continue;
      }
      finger += STEP_PX;
      await touch(cdp, "touchMove", x, finger);
      await frames(page);
      const now = await sample(page);
      if (last !== null && now !== null) {
        // A view that jumped clean away leaves nothing in both samples to measure by.
        if (moves(last.tops, now.tops).length === 0) {
          strays.push(`step ${String(step)}: nothing in view before was in view after`);
        }
        const wrong = moves(last.tops, now.tops).filter((d) => Math.abs(d - STEP_PX) > 1);
        if (wrong.length > 0) {
          const was = last;
          const grew = Object.keys(now.heights)
            .filter(
              (key) =>
                key in was.heights &&
                Math.abs((now.heights[key] ?? 0) - (was.heights[key] ?? 0)) > 1,
            )
            .map(
              (key) =>
                `${String(parseInt(key.slice(-12), 16))}:${String(Math.round(was.heights[key] ?? 0))}->${String(Math.round(now.heights[key] ?? 0))}`,
            );
          strays.push(
            `step ${String(step)}: moved ${[...new Set(wrong.map(Math.round))].join(", ")}px, not ${String(STEP_PX)}; grew ${grew.join(" ")}; articles ${String(was.count)}->${String(now.count)} first ${String(parseInt(was.first.slice(-12), 16))}->${String(parseInt(now.first.slice(-12), 16))} top ${String(Math.round(was.scrollTop))}->${String(Math.round(now.scrollTop))}`,
          );
        }
        if (now.seamShows) {
          seams.push(step);
        }
      }
      last = now;
      reached = now ?? reached;
    }
    await page.waitForTimeout(120);
    await touch(cdp, "touchEnd", x, finger);

    // The reader got a long way back, or the test would prove little.
    const oldestSeen = Math.min(
      ...Object.keys(reached?.tops ?? {}).map((key) => parseInt(key.slice(-12), 16)),
    );
    expect.soft(seams, "steps at which the top of what was loaded came into view").toEqual([]);
    expect.soft(strays, "times the view moved other than with the finger").toEqual([]);
    expect(oldestSeen).toBeLessThan(COUNT - 80);
  });
}

// A finger reports where it is in fractions of a pixel, and moves a fraction at a time on a
// dense screen, while a box holds its position in whole pixels: WebKit truncates what it is set
// to, Chromium rounds it. A list that built each move on what its box read back gained up to a
// pixel a move on iOS, running ahead of the finger, and here loses each move short of half a
// pixel, standing still under it.
test("a finger moving a fraction of a pixel at a time moves the list as far as it goes, with the list scrolling itself as on iOS", async ({
  page,
  browserName,
  isMobile,
}) => {
  test.skip(
    !isMobile || browserName !== "chromium",
    "a finger is driven through Chromium's protocol",
  );
  test.setTimeout(120_000);
  await asIos(page);
  await signInToWorld(page, serveHistory);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.locator(`article[data-message-id="${id(COUNT)}"]`)).toBeVisible();
  await page.waitForTimeout(1500);

  const cdp = await page.context().newCDPSession(page);
  const box = await page.locator("[data-message-list]").first().boundingBox();
  expect(box).not.toBeNull();
  const x = (box?.x ?? 0) + (box?.width ?? 400) / 2;
  let finger = (box?.y ?? 100) + 40;
  await touch(cdp, "touchStart", x, finger);
  finger += 20;
  await touch(cdp, "touchMove", x, finger);
  await frames(page);
  const before = await sample(page);
  const distance = 120;
  const step = 0.375;
  for (let moved = 0; moved < distance; moved += step) {
    finger += step;
    await touch(cdp, "touchMove", x, finger);
  }
  await frames(page);
  const after = await sample(page);
  await page.waitForTimeout(120);
  await touch(cdp, "touchEnd", x, finger);

  const both = moves(before?.tops ?? {}, after?.tops ?? {});
  expect(both.length, "messages in view before and after").toBeGreaterThan(0);
  for (const moved of both) {
    expect(
      Math.abs(moved - distance),
      `moved ${String(moved)}px, not ${String(distance)}`,
    ).toBeLessThan(1.5);
  }
});

// A row above the view changes size in a task of its own, a picture or a card taking its room,
// and the finger moves before the next frame tells the rows' observer: a fling's frames run
// before the observer is told, too. A move that noted the row it keeps still where that row now
// stood left the observer nothing to absorb, and the view jumped by the change.
test("a row above the view growing just before a move does not move what is in view, with the list scrolling itself as on iOS", async ({
  page,
  browserName,
  isMobile,
}) => {
  test.skip(
    !isMobile || browserName !== "chromium",
    "a finger is driven through Chromium's protocol",
  );
  test.setTimeout(120_000);
  await asIos(page);
  await signInToWorld(page, serveHistory);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.locator(`article[data-message-id="${id(COUNT)}"]`)).toBeVisible();
  await page.waitForTimeout(1500);

  const cdp = await page.context().newCDPSession(page);
  const box = await page.locator("[data-message-list]").first().boundingBox();
  expect(box).not.toBeNull();
  const x = (box?.x ?? 0) + (box?.width ?? 400) / 2;
  let finger = (box?.y ?? 100) + 40;
  await touch(cdp, "touchStart", x, finger);
  finger += 20;
  await touch(cdp, "touchMove", x, finger);
  await frames(page);
  const before = await sample(page);
  const step = 10;
  // In one task: the row just above the view grows, and the finger moves.
  const grown = await page.evaluate(
    ({ x, y }) => {
      const list = document.querySelector<HTMLElement>("[data-message-list]");
      if (list === null) {
        return false;
      }
      const top = list.getBoundingClientRect().top;
      const above = [...list.querySelectorAll<HTMLElement>("article[data-message-id]")]
        .filter((article) => article.getBoundingClientRect().bottom <= top)
        .pop();
      if (above === undefined) {
        return false;
      }
      above.style.paddingBottom = "105px";
      const touch = new Touch({ identifier: 1, target: list, clientX: x, clientY: y });
      list.dispatchEvent(
        new TouchEvent("touchmove", {
          bubbles: true,
          cancelable: true,
          touches: [touch],
          targetTouches: [touch],
          changedTouches: [touch],
        }),
      );
      return true;
    },
    { x, y: finger + step },
  );
  expect(grown, "a row above the view to grow").toBe(true);
  await frames(page);
  const after = await sample(page);
  await touch(cdp, "touchEnd", x, finger + step);

  const both = moves(before?.tops ?? {}, after?.tops ?? {});
  expect(both.length, "messages in view before and after").toBeGreaterThan(0);
  for (const moved of both) {
    expect(Math.abs(moved - step), `moved ${String(moved)}px, not ${String(step)}`).toBeLessThan(
      1.5,
    );
  }
});

for (const { name, ios } of SCROLLERS) {
  test(`flicking back through a long history keeps ahead of the reader${name}`, async ({
    page,
    browserName,
    isMobile,
  }) => {
    test.skip(
      !isMobile || browserName !== "chromium",
      "a finger is driven through Chromium's protocol",
    );
    test.setTimeout(300_000);
    if (ios) {
      await asIos(page);
    }
    await signInToWorld(page, serveHistory);
    await page
      .getByRole("grid", { name: "Channels" })
      .first()
      .getByText("general", { exact: true })
      .click();
    await expect(page.locator(`article[data-message-id="${id(COUNT)}"]`)).toBeVisible();
    await page.waitForTimeout(1500);
    // Every frame: whether the top of what is loaded, where a page is awaited, is in view. And
    // the longest the main thread was held, which a page's render does: a finger is frozen for
    // as long.
    await page.evaluate(() => {
      const record = window as unknown as {
        seamFrames: number;
        frames: number;
        longestTask: number;
      };
      record.seamFrames = 0;
      record.frames = 0;
      record.longestTask = 0;
      new PerformanceObserver((entries) => {
        for (const entry of entries.getEntries()) {
          record.longestTask = Math.max(record.longestTask, entry.duration);
        }
      }).observe({ type: "longtask" });
      const tick = () => {
        const scroller = document.querySelector("article")?.closest("[data-message-list]");
        const seam = scroller?.querySelector('p[aria-live="polite"]');
        if (scroller instanceof HTMLElement && seam !== null && seam !== undefined) {
          const view = scroller.getBoundingClientRect();
          const rect = seam.getBoundingClientRect();
          record.frames += 1;
          if (rect.bottom > view.top && rect.top < view.bottom) {
            record.seamFrames += 1;
          }
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    const cdp = await page.context().newCDPSession(page);
    const centre = await page.evaluate(() => {
      const rect = document
        .querySelector("article")
        ?.closest("[data-message-list]")
        ?.getBoundingClientRect();
      return rect === undefined
        ? { x: 200, y: 400 }
        : { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    });
    // A reader flicks back, glances at where they landed, and flicks again.
    for (let flick = 0; flick < FLICKS; flick++) {
      // A quick stroke released while moving, which the browser carries on as a fling.
      await touch(cdp, "touchStart", centre.x, centre.y - 200);
      for (let move = 1; move <= 8; move++) {
        await touch(cdp, "touchMove", centre.x, centre.y - 200 + move * 50);
        await page.waitForTimeout(4);
      }
      await touch(cdp, "touchEnd", centre.x, centre.y + 200);
      await page.waitForTimeout(500);
    }
    const { seamFrames, frames, longestTask } = await page.evaluate(() => {
      const record = window as unknown as {
        seamFrames: number;
        frames: number;
        longestTask: number;
      };
      return {
        seamFrames: record.seamFrames,
        frames: record.frames,
        longestTask: record.longestTask,
      };
    });
    console.log(`longest task ${String(Math.round(longestTask))}ms`);
    const oldest = await page.evaluate(() =>
      Math.min(
        ...Array.from(document.querySelectorAll<HTMLElement>("article[data-message-id]"), (a) =>
          parseInt((a.dataset.messageId ?? "").slice(-12), 16),
        ),
      ),
    );
    expect(oldest, "how far back the flicks reached").toBeLessThan(COUNT - 200);
    // Flicks in quick succession gather speed, as on a phone, until one crosses a page's worth
    // of pictures before it has arrived; the reader may then glimpse where it is awaited, for a
    // moment and seldom.
    expect(
      seamFrames / frames,
      `share of frames in which the top of what was loaded was in view (${String(seamFrames)} of ${String(frames)})`,
    ).toBeLessThan(SEAM_SHARE);
  });
}

/**
 * A history a little longer than the window the list keeps, newest first: short lines from one
 * person, moments apart, so all of it is only a few screens tall.
 */
const SHORT_COUNT = 320;
const shortHistory = Array.from({ length: SHORT_COUNT }, (_, i) => {
  const n = SHORT_COUNT - i;
  const { record } = messageOf(n);
  return {
    record: {
      ...record,
      author: bob,
      timestamp: new Date(Date.UTC(2026, 0, 1) + n * 5_000).toISOString(),
      content: `Line ${String(n)}`,
      attachments: [],
    },
    picture: null,
  };
});

test("the Jump to latest pill offers itself a screen above the newest, and goes when they are near", async ({
  page,
}) => {
  await signInToWorld(page, (p) => serveHistory(p, shortHistory));
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  const newest = page.locator(`article[data-message-id="${id(SHORT_COUNT)}"]`);
  await expect(newest).toBeVisible();
  const pill = page.getByRole("button", { name: "Jump to latest" });
  await expect(pill).toHaveCount(0);
  // The newest messages are still in the window; the view alone is far from them.
  const list = page.locator("[data-message-list]");
  const screensUp = (screens: number) =>
    list.evaluate((element, n) => {
      element.scrollTop = element.scrollHeight - element.clientHeight * (1 + n);
    }, screens);
  await screensUp(2);
  await expect(pill).toBeVisible();
  await screensUp(0.5);
  await expect(pill).toHaveCount(0);
  await screensUp(1.5);
  await expect(pill).toBeVisible();
  // Reached from the keyboard, it stays where the view is; it is not where it would be in
  // the flow of the content, at the end.
  const before = await list.evaluate((element) => element.scrollTop);
  await pill.focus();
  await expect(pill).toBeFocused();
  expect(await list.evaluate((element) => element.scrollTop)).toBe(before);
  await page.keyboard.press("Enter");
  await expect(newest).toBeVisible();
  await expect(pill).toHaveCount(0);
});

test("reading back through a history a little longer than the window keeps what is in view", async ({
  page,
  browserName,
  isMobile,
}) => {
  test.skip(
    !isMobile || browserName !== "chromium",
    "a finger is driven through Chromium's protocol",
  );
  test.setTimeout(120_000);
  // Each page read at one end of a full window drops messages from the other: the newest,
  // while the reader is still among them, or, read back at the bottom, the oldest above the
  // view, again and again.
  const reads: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith(`/channels/${general}/messages`) && url.searchParams.has("after")) {
      reads.push(url.search);
    }
  });
  // A tall phone, on which the whole of what the window holds is within reading ahead.
  await page.setViewportSize({ width: 430, height: 1400 });
  await signInToWorld(page, (p) => serveHistory(p, shortHistory));
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  const newest = page.locator(`article[data-message-id="${id(SHORT_COUNT)}"]`);
  await expect(newest).toBeVisible();
  // History pages in ahead of a reader who has not yet moved, and what they are reading stays.
  await page.waitForTimeout(3000);
  await expect(
    newest,
    "the newest message, still in view once history has paged in",
  ).toBeInViewport();
  // Every frame: whether a message that was in view in the last one has left the page.
  await page.evaluate(() => {
    const record = window as unknown as { vanished: string[] };
    record.vanished = [];
    let inView: string[] = [];
    const tick = () => {
      const scroller = document.querySelector("article")?.closest("[data-message-list]");
      if (scroller instanceof HTMLElement) {
        for (const id of inView) {
          if (scroller.querySelector(`article[data-message-id="${id}"]`) === null) {
            record.vanished.push(id.slice(-4));
          }
        }
        const view = scroller.getBoundingClientRect();
        inView = Array.from(scroller.querySelectorAll<HTMLElement>("article[data-message-id]"))
          .filter((article) => {
            const rect = article.getBoundingClientRect();
            return rect.bottom > view.top && rect.top < view.bottom;
          })
          .map((article) => article.dataset.messageId ?? "");
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  const cdp = await page.context().newCDPSession(page);
  const box = await page
    .locator("article")
    .first()
    .evaluate((article) => {
      const rect = article.closest("[data-message-list]")?.getBoundingClientRect();
      return rect === undefined ? null : { x: rect.x + rect.width / 2, y: rect.top + 60 };
    });
  const x = box?.x ?? 200;
  const y = box?.y ?? 160;
  const strays: string[] = [];
  for (let stroke = 0; stroke < 40; stroke++) {
    // A gentle stroke back through history, and a pause to read, in which nothing may move.
    await touch(cdp, "touchStart", x, y);
    for (let move = 1; move <= 10; move++) {
      await touch(cdp, "touchMove", x, y + move * 15);
      await page.waitForTimeout(16);
    }
    await page.waitForTimeout(200);
    await touch(cdp, "touchEnd", x, y + 150);
    const resting = await sample(page);
    await page.waitForTimeout(800);
    const rested = await sample(page);
    if (resting !== null && rested !== null) {
      const gone = Object.keys(resting.tops).filter((key) => !(key in rested.tops));
      const drift = moves(resting.tops, rested.tops).filter((d) => Math.abs(d) > 1);
      if (gone.length > 0 || drift.length > 0) {
        strays.push(
          `after stroke ${String(stroke)}: ${String(gone.length)} gone, moved ${drift.join(", ")}`,
        );
      }
    }
  }
  const vanished = await page.evaluate(
    () => (window as unknown as { vanished: string[] }).vanished,
  );
  expect(vanished, "messages in view that left the page").toEqual([]);
  expect(strays, "times the view changed while the reader rested").toEqual([]);
  expect(reads, "reads of newer messages while reading back").toEqual([]);
});

for (const { name, ios } of SCROLLERS) {
  test(`a drag that starts on a picture scrolls, and a tap on it opens it${name}`, async ({
    page,
    browserName,
    isMobile,
  }) => {
    test.skip(
      !isMobile || browserName !== "chromium",
      "a finger is driven through Chromium's protocol",
    );
    if (ios) {
      await asIos(page);
    }
    // Nothing pans the list but the list itself, so nothing cancels a press under a finger that
    // drags; the list must, or lifting the finger over a tall picture would open it.
    await signInToWorld(page, serveHistory);
    await page
      .getByRole("grid", { name: "Channels" })
      .first()
      .getByText("general", { exact: true })
      .click();
    await expect(page.locator(`article[data-message-id="${id(COUNT)}"]`)).toBeVisible();
    await page.waitForTimeout(1000);
    const list = page.locator("[data-message-list]");
    const view = await list.boundingBox();
    const pictures = list.getByRole("button", { name: "Open image" });
    // A picture whose middle is in view; the content moves with the finger, so a drag from
    // there stays on it.
    let picture: { x: number; y: number } | null = null;
    for (let i = (await pictures.count()) - 1; i >= 0 && picture === null; i--) {
      const box = await pictures.nth(i).boundingBox();
      if (box === null || view === null) {
        continue;
      }
      const middle = box.y + box.height / 2;
      if (middle > view.y + 20 && middle < view.y + view.height - 20) {
        picture = { x: box.x + box.width / 2, y: middle };
      }
    }
    expect(picture, "a picture in view").not.toBeNull();
    const x = picture?.x ?? 0;
    const y = picture?.y ?? 0;
    const cdp = await page.context().newCDPSession(page);
    const before = await sample(page);
    await touch(cdp, "touchStart", x, y);
    for (let move = 1; move <= 6; move++) {
      await touch(cdp, "touchMove", x, y + move * 12);
      await frames(page);
    }
    await page.waitForTimeout(120);
    await touch(cdp, "touchEnd", x, y + 72);
    await page.waitForTimeout(300);
    await expect(page.getByRole("button", { name: "Close gallery" })).toHaveCount(0);
    const after = await sample(page);
    const moved = before !== null && after !== null ? moves(before.tops, after.tops) : [];
    expect(moved.length, "something in view to measure").toBeGreaterThan(0);
    // The browser's own pan, where it scrolls the list, starts past a slop it does not count.
    for (const by of moved) {
      expect(by).toBeLessThanOrEqual(72 + 1);
      expect(by).toBeGreaterThanOrEqual(72 - 16 - 1);
    }
    // A tap, which moves nothing, opens the picture.
    await touch(cdp, "touchStart", x, y);
    await touch(cdp, "touchEnd", x, y);
    await expect(page.getByRole("button", { name: "Close gallery" })).toBeVisible();
  });
}

for (const { name, ios } of SCROLLERS) {
  test(`a jump to a message lands it in the middle, and a jump to the latest at the bottom${name}`, async ({
    page,
    isMobile,
  }) => {
    test.skip(!isMobile, "measured on the phones, Safari's among them, which rounds positions");
    test.setTimeout(180_000);
    if (ios) {
      await asIos(page);
    }
    // Pictures arrive for seconds after either jump, above and below what was jumped to, and
    // the list must keep what it landed on exactly where it landed.
    await signInToWorld(page, serveHistory);
    // The page is reloaded below; the sign-in must have been kept first, which it is a moment
    // after signing in.
    await page.waitForFunction(() => localStorage.getItem("aspen.session") !== null);
    const misses: string[] = [];
    for (const n of [COUNT - 130, COUNT - 260, COUNT - 410, COUNT - 55, COUNT - 333]) {
      await page.goto(`/communities/${community}/channels/${general}/messages/${id(n)}`);
      const target = page.locator(`article[data-message-id="${id(n)}"]`);
      // Says what the page showed instead, should the message never come.
      await target.waitFor({ timeout: 60_000 }).catch(async () => {
        const shown = (await page.locator("body").innerText()).slice(0, 300).replace(/\s+/g, " ");
        throw new Error(`no message ${String(n)} after the jump; the page shows: ${shown}`);
      });
      await expect(target).toBeVisible();
      // Where it landed, as soon as it has; and where it is once every picture has arrived.
      await page.waitForTimeout(100);
      const landed = await target.evaluate((element) => element.getBoundingClientRect().top);
      await page.waitForTimeout(3000);
      const settled = await target.evaluate((element) => {
        const list = element.closest("[data-message-list]");
        const view = list?.getBoundingClientRect();
        const rect = element.getBoundingClientRect();
        return {
          top: rect.top,
          centred:
            view === undefined ? NaN : rect.top + rect.height / 2 - (view.top + view.height / 2),
        };
      });
      if (Math.abs(settled.top - landed) > 1) {
        misses.push(
          `message ${String(n)} moved ${String(Math.round(settled.top - landed))}px after landing`,
        );
      }
      if (Math.abs(settled.centred) > 1) {
        misses.push(
          `message ${String(n)} sits ${String(Math.round(settled.centred))}px off the middle`,
        );
      }
      await page.getByRole("button", { name: "Jump to latest" }).click();
      const newest = page.locator(`article[data-message-id="${id(COUNT)}"]`);
      await expect(newest).toBeVisible({ timeout: 60_000 });
      await page.waitForTimeout(3000);
      const gap = await newest.evaluate((element) => {
        const list = element.closest("[data-message-list]");
        const view = list?.getBoundingClientRect();
        const content = list?.firstElementChild;
        const padding =
          content === null || content === undefined
            ? 0
            : parseFloat(getComputedStyle(content).paddingBottom);
        return view === undefined
          ? NaN
          : view.bottom - padding - element.getBoundingClientRect().bottom;
      });
      if (Math.abs(gap) > 1) {
        misses.push(
          `after message ${String(n)}, the latest sits ${String(Math.round(gap))}px off the bottom`,
        );
      }
    }
    expect(misses).toEqual([]);
  });
}
