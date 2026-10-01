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
const STEPS = 3000;

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
async function serveHistory(page: Page) {
  await page.route(`**/api/v1/channels/${general}/messages*`, async (route) => {
    const url = new URL(route.request().url());
    const limit = Number(url.searchParams.get("limit") ?? "50");
    const before = url.searchParams.get("before");
    const after = url.searchParams.get("after");
    let slice: typeof history;
    if (before !== null) {
      await new Promise((resolve) => setTimeout(resolve, PAGE_DELAY_MS));
      const start = history.findIndex((m) => (m.record.id as string) < before);
      slice = start === -1 ? [] : history.slice(start, start + limit);
    } else if (after !== null) {
      const newer = history.filter((m) => (m.record.id as string) > after).reverse();
      slice = newer.slice(0, limit).reverse();
    } else {
      slice = history.slice(0, limit);
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

test("reading slowly back through a long history never waits and never moves by itself", async ({
  page,
  browserName,
  isMobile,
}) => {
  test.skip(
    !isMobile || browserName !== "chromium",
    "a finger is driven through Chromium's protocol",
  );
  test.setTimeout(600_000);
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
        const drift = moves(resting.tops, rested.tops).filter((d) => Math.abs(d) > 1);
        if (drift.length > 0) {
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
      const wrong = moves(last.tops, now.tops).filter((d) => Math.abs(d - STEP_PX) > 1);
      if (wrong.length > 0) {
        const was = last;
        const grew = Object.keys(now.heights)
          .filter(
            (key) =>
              key in was.heights && Math.abs((now.heights[key] ?? 0) - (was.heights[key] ?? 0)) > 1,
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
  expect(oldestSeen).toBeLessThan(COUNT - 120);
});
