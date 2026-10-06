import type { Locator, Page } from "@playwright/test";
import { administration } from "./world/administration";
import { answer } from "./world/answer";
import { me, PIXEL_PNG, PIXEL_PNG_BYTES } from "./world/fixtures";
import { lunch } from "./world/poll";
import type { Publish } from "./world/reply";

/**
 * A small signed-in world for specs that need more than an empty account: one community with a
 * text channel, a voice channel, and a category; a conversation in the text channel with a
 * thread and an echoed reply, then a poll of Bob's that takes write-ins; and a one-to-one DM.
 * The caller has read #general up to Bob's "Sounds good" (`lastReadText`) and has not read the
 * DM. #roadmap shows as unread without any history, for checking the channel list alone;
 * #ideas, beside it in Planning, is read. The Archive category is empty. Every API read the app makes about it is
 * answered from here, and anything else is refused with a Problem naming the request, so a
 * spec fails loudly rather than waiting on a request nobody answers.
 */

export {
  HISTORY_DELAY_MS,
  archive,
  bob,
  community,
  customEmojiId,
  customEmojiName,
  dm,
  dmMessageId,
  general,
  gifPageLink,
  helper,
  ideas,
  inviteCode,
  lastReadText,
  lounge,
  lunchPoll,
  me,
  missingPictureLink,
  missingPictureMessageId,
  organiserRole,
  organiserRoleWith,
  ownText,
  planning,
  pollQuestion,
  reactedText,
  roadmap,
  singleReactions,
  starterText,
  thread,
} from "./world/fixtures";
export { standingInvite } from "./world/administration";
export { friendlyDeployment, rekeyedDeployment } from "./world/federation";
export type { Publish } from "./world/reply";

/**
 * Answers the event stream's `identify` with `ready`, then sends only what the returned
 * `publish` is given.
 */
async function events(page: Page): Promise<Publish> {
  let send: (frame: string) => void = () => undefined;
  let sequence = 0;
  await page.routeWebSocket(/\/api\/v1\/events(\?.*)?$/, (ws) => {
    send = (frame) => {
      ws.send(frame);
    };
    ws.onMessage((frame) => {
      const parsed: unknown = JSON.parse(String(frame));
      if (
        typeof parsed === "object" &&
        parsed !== null &&
        "type" in parsed &&
        parsed.type === "identify"
      ) {
        ws.send(JSON.stringify({ type: "ready", userId: me, resumed: false }));
      }
    });
  });
  return (event) => {
    sequence += 1;
    send(JSON.stringify({ type: "event", sequence, event }));
  };
}

/** Stubs the world and signs in through the form, as a user would. */
export async function signInToWorld(
  page: Page,
  /** Routes that answer before the world's own, registered after it so they win. */
  before?: (page: Page) => Promise<void>,
): Promise<Publish> {
  const publish = await events(page);
  const poll = lunch(publish);
  const admin = administration();
  const blocks = new Set<string>();
  /** The community's standing bans, by user, as the tests make and lift them. */
  const bans = new Map<string, Record<string, unknown>>();
  await page.route(/\/api\/v1\//, (route) => answer(route, poll, publish, admin, blocks, bans));
  await page.route(PIXEL_PNG, (route) =>
    route.fulfill({ contentType: "image/png", body: PIXEL_PNG_BYTES }),
  );
  await before?.(page);
  await page.goto("/");
  await page.getByLabel("Username").fill("kate");
  await page.getByLabel("Password").fill("hunter22");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  // Signed in once the session is kept, which is a moment after the button: a test that
  // reloads the page before then lands on the sign-in screen, on a busy machine especially.
  await page.waitForFunction(() => localStorage.getItem("aspen.session") !== null);
  return publish;
}

/**
 * Waits for every animation that ends to end, so what is checked (colour contrast, above all)
 * is what stays on screen rather than a frame of a fade. Endless ones, such as a skeleton's
 * pulse, are left running.
 */
export async function settleAnimations(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
}

/**
 * Sends what is in the message box as the device would: Enter on a computer, the Send button
 * on a touch screen, whose keyboard writes a new line with Enter.
 */
export async function submit(page: Page): Promise<void> {
  const touchOnly = await page.evaluate(
    () => window.matchMedia("(hover: none) and (pointer: coarse)").matches,
  );
  if (touchOnly) {
    await page.getByRole("button", { name: "Send", exact: true }).click();
  } else {
    await page.keyboard.press("Enter");
  }
}

/**
 * Holds a finger on `target` for longer than a long press takes, without moving it: a real
 * touch through Chromium's protocol, and elsewhere, which Playwright gives no held touch for,
 * the pointer events a touch raises, dispatched to the element. Answers where it pressed.
 */
export async function longPress(page: Page, target: Locator): Promise<{ x: number; y: number }> {
  // Still before it is pressed, as Playwright's own tap waits for: a press on an element
  // that moves (a list settling to its bottom as its history loads) is cancelled by the
  // scroll, as a finger's would be.
  let box = await target.boundingBox();
  for (let tries = 0; tries < 50; tries++) {
    await target.scrollIntoViewIfNeeded();
    await page.waitForTimeout(100);
    const again = await target.boundingBox();
    const viewport = page.viewportSize();
    if (
      box !== null &&
      again !== null &&
      again.x === box.x &&
      again.y === box.y &&
      again.y >= 0 &&
      (viewport === null || again.y + again.height <= viewport.height)
    ) {
      break;
    }
    box = again;
  }
  if (box === null) {
    throw new Error("nothing to press");
  }
  const point = { x: box.x + box.width / 2, y: box.y + Math.min(box.height / 2, 20) };
  if (page.context().browser()?.browserType().name() === "chromium") {
    const client = await page.context().newCDPSession(page);
    await client.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] });
    await page.waitForTimeout(700);
    await client.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await client.detach();
    return point;
  }
  const pointer = {
    pointerType: "touch",
    pointerId: 1,
    isPrimary: true,
    clientX: point.x,
    clientY: point.y,
    bubbles: true,
    cancelable: true,
    composed: true,
  };
  await target.dispatchEvent("pointerdown", { ...pointer, button: 0, buttons: 1 });
  await page.waitForTimeout(700);
  await target.dispatchEvent("pointerup", { ...pointer, button: 0, buttons: 0 });
  return point;
}
