import { expect, test, type Page } from "@playwright/test";
import { community, general, longPress, me, signInToWorld, submit, type Publish } from "./world";
import { message } from "./world/fixtures";

/**
 * Starting a thread, against the stubbed world: "Reply in thread" on a message with none opens
 * the thread's panel without making anything, and the first reply makes the thread. A first
 * reply held for its preview and then dropped takes its thread with it, and waits to be sent
 * again on the message it replied to.
 */

/** Bob's message in #general, which starts no thread. */
const starterText = "Sounds good. See everyone at ten";
const starter = "0190f0a0-0000-7000-8001-000000000204";
const madeThread = "0190f0a0-0000-7000-8000-0000000000f1";
const firstReply = message(960, me, "Count me in", 0, { channelId: madeThread });
const madeThreadRecord = {
  id: madeThread,
  parentChannel: general,
  starterMessage: starter,
  replyCount: 1,
  lastReplyAt: firstReply.timestamp,
  recipients: [],
  parentCategory: null,
  community,
  name: "",
  sortIndex: 0,
  ty: "thread",
};

/** A first reply the server held for its picture's preview. */
const heldReply = {
  id: "0190f0a0-0000-7000-8000-0000000000f2",
  channelId: madeThread,
  content: "Count me in",
  attachments: ["0190f0a0-0000-7000-8000-0000000000f3"],
  echoToParent: false,
  heldAt: firstReply.timestamp,
};

/** What the app asked of the server about the thread, as the stand-in answered it. */
interface Asked {
  opened: number;
  replies: unknown[];
  publish: Publish;
}

/**
 * Signs in to a world whose stand-in posts first replies to the starter, holding the first
 * `holding` of them for their previews.
 */
async function signIn(page: Page, holding = 0): Promise<Asked> {
  const asked: Omit<Asked, "publish"> = { opened: 0, replies: [] };
  const publish = await signInToWorld(page, async (page) => {
    await page.route(/\/api\/v1\/messages\/[^/]+\/thread$/, (route) => {
      asked.opened += 1;
      return route.fulfill({ status: 500 });
    });
    await page.route(new RegExp(`/api/v1/messages/${starter}/thread/messages$`), (route) => {
      asked.replies.push(route.request().postDataJSON());
      return asked.replies.length <= holding
        ? route.fulfill({ status: 202, json: heldReply })
        : route.fulfill({ status: 201, json: firstReply });
    });
    await page.route(new RegExp(`/api/v1/channels/${madeThread}$`), (route) =>
      route.fulfill({ json: madeThreadRecord }),
    );
    await page.route(new RegExp(`/api/v1/channels/${madeThread}/messages(\\?.*)?$`), (route) =>
      route.fulfill({
        json: { data: asked.replies.length > holding ? [firstReply] : [], included: { users: [] } },
      }),
    );
  });
  return Object.assign(asked, { publish });
}

async function replyInThread(page: Page, isMobile: boolean) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText("general", { exact: true }).first().click();
  const article = page.locator("article").filter({ hasText: starterText }).last();
  if (isMobile) {
    await longPress(page, article.locator(".message-body"));
    const sheet = page.getByRole("dialog", { name: "Message actions" });
    await sheet.getByRole("button", { name: "Reply in thread" }).tap();
    // The sheet holds focus until it has gone.
    await expect(sheet).toHaveCount(0);
  } else {
    await article.hover();
    await article
      .getByRole("group", { name: "Message actions" })
      .getByRole("button", { name: "Reply in thread" })
      .click();
  }
}

const panel = (page: Page) => page.locator("section[aria-label='Thread']");

test("replying in a thread makes nothing until the first reply is sent", async ({
  page,
  isMobile,
}) => {
  const asked = await signIn(page);
  await replyInThread(page, isMobile);
  await expect(page).toHaveURL(new RegExp(`/messages/${starter}/thread$`));
  await expect(panel(page).getByText("No replies yet.")).toBeVisible();
  await expect(panel(page).getByText(starterText)).toBeVisible();
  // Closed without a reply, nothing was made.
  await panel(page).getByRole("button", { name: "Close thread" }).click();
  await expect(panel(page)).toHaveCount(0);
  expect(asked.opened).toBe(0);
  expect(asked.replies).toEqual([]);

  await replyInThread(page, isMobile);
  await panel(page).getByRole("textbox", { name: "Message" }).fill("Count me in");
  await submit(page);
  await expect(page).toHaveURL(new RegExp(`/threads/${madeThread}$`));
  await expect(panel(page).getByText("Count me in")).toBeVisible();
  expect(asked.opened).toBe(0);
  expect(asked.replies).toEqual([{ content: "Count me in", attachments: [], mayHold: true }]);
});

test("a dropped first reply takes its thread, and sent again makes it anew", async ({
  page,
  isMobile,
}) => {
  const asked = await signIn(page, 1);
  await replyInThread(page, isMobile);
  await panel(page).getByRole("textbox", { name: "Message" }).fill("Count me in");
  await submit(page);
  // Held, it made the thread, and waits there.
  await expect(page).toHaveURL(new RegExp(`/threads/${madeThread}$`));
  await expect(panel(page).getByText(starterText)).toBeVisible();
  await expect(panel(page).locator(`[data-held-message="${heldReply.id}"]`)).toBeVisible();
  // Dropped, it takes the thread with it, as the server says in one go.
  asked.publish({
    serverEvent: "heldMessageFailed",
    held: heldReply.id,
    channel: madeThread,
    detail: "You can no longer post here.",
  });
  asked.publish({ serverEvent: "message", type: "update", id: starter, thread: null });
  asked.publish({ serverEvent: "channel", type: "delete", id: madeThread });
  await expect(page).toHaveURL(new RegExp(`/messages/${starter}/thread$`));
  await expect(panel(page).getByText("No replies yet.")).toBeVisible();
  const dropped = panel(page).locator(`[data-held-message="${heldReply.id}"]`);
  await expect(dropped.getByText("Not sent: You can no longer post here.")).toBeVisible();
  await dropped.getByRole("button", { name: "Send again" }).click();
  await expect(page).toHaveURL(new RegExp(`/threads/${madeThread}$`));
  await expect(panel(page).getByText("Count me in")).toBeVisible();
  await expect(dropped).toHaveCount(0);
  expect(asked.opened).toBe(0);
  expect(asked.replies).toEqual([
    { content: "Count me in", attachments: [], mayHold: true },
    { content: "Count me in", attachments: heldReply.attachments, mayHold: true },
  ]);
});
