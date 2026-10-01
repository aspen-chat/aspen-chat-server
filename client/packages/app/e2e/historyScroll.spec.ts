import { expect, test, type CDPSession, type Page } from "@playwright/test";
import { bob, general, me, signInToWorld } from "./world";

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
    let slice: typeof history;
    if (before !== null) {
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

/** What the list shows: where each message in view sits, and whether the top of what is loaded shows. */
function sample(page: Page) {
  return page.evaluate(() => {
    const scroller = document.querySelector("article")?.closest(".overflow-y-auto");
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

/**
 * The browsers the slow test reads in, by what they do when content above the view changes:
 * anchor it while the list moves, anchor it but take no change while the list moves (as iOS,
 * which scrolls apart from the page), or not anchor it at all (as older Safari).
 */
const SCROLLERS = [
  { name: "", anchoring: true, ios: false },
  { name: ", where changes wait for rest (as iOS)", anchoring: true, ios: true },
  { name: ", where the browser anchors nothing (as older Safari)", anchoring: false, ios: false },
];

for (const { name, anchoring, ios } of SCROLLERS) {
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
      // A finger cannot be driven through WebKit here, so Chromium is taken for iOS by the
      // property only iOS knows, and the list then shows no change while it moves.
      await page.addInitScript(() => {
        const supports = CSS.supports.bind(CSS) as (property: string, value: string) => boolean;
        CSS.supports = ((property: string, value: string) =>
          property === "-webkit-touch-callout" || supports(property, value)) as typeof CSS.supports;
      });
    }
    if (!anchoring) {
      // Chromium is made to anchor nothing: the list's own holding is then all there is.
      await page.addInitScript(() => {
        document.addEventListener("DOMContentLoaded", () => {
          const style = document.createElement("style");
          style.textContent = "* { overflow-anchor: none !important; }";
          document.head.append(style);
        });
      });
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
        const rect = article.closest(".overflow-y-auto")?.getBoundingClientRect();
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
        // The finger lifts and comes back to the top, as a reader's does; the list must stay
        // where it was left while it is away.
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

test("flicking back through a long history keeps ahead of the reader", async ({
  page,
  browserName,
  isMobile,
}) => {
  test.skip(
    !isMobile || browserName !== "chromium",
    "a finger is driven through Chromium's protocol",
  );
  test.setTimeout(300_000);
  await signInToWorld(page, serveHistory);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.locator(`article[data-message-id="${id(COUNT)}"]`)).toBeVisible();
  await page.waitForTimeout(1500);
  // Every frame: whether the top of what is loaded, where a page is awaited, is in view.
  await page.evaluate(() => {
    const record = window as unknown as { seamFrames: number; frames: number };
    record.seamFrames = 0;
    record.frames = 0;
    const tick = () => {
      const scroller = document.querySelector("article")?.closest(".overflow-y-auto");
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
      ?.closest(".overflow-y-auto")
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
  const { seamFrames, frames } = await page.evaluate(() => {
    const record = window as unknown as { seamFrames: number; frames: number };
    return { seamFrames: record.seamFrames, frames: record.frames };
  });
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
      const scroller = document.querySelector("article")?.closest(".overflow-y-auto");
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
      const rect = article.closest(".overflow-y-auto")?.getBoundingClientRect();
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
