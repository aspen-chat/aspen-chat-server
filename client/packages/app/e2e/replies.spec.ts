import { expect, test, type Locator, type Page } from "@playwright/test";
import { bob, general, longPress, me, ownText, signInToWorld, submit } from "./world";
import { message, messageId } from "./world/fixtures";

/**
 * Replies, against the stubbed world: a message's Reply makes the next message a reply to it,
 * tagging its author unless that is turned off; the reply shows what it answers above it, which
 * jumps to it; and a reply whose original is deleted says so.
 */

/** Bob's message in #general that the tests reply to. */
const answeredText = "Sounds good. See everyone at ten";
const answered = messageId(204);
const bobName = /^@?Bob With A Rather Long Display Name$/;

const box = (page: Page) => page.getByRole("textbox", { name: "Message" });
const row = (page: Page, text: string) => page.locator("article").filter({ hasText: text }).last();

/** Opens #general, by way of the channel list on a phone. */
async function openGeneral(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText("general", { exact: true }).first().click();
}

/** Chooses Reply among a message's actions: the row's on a computer, the sheet's on a phone. */
async function reply(page: Page, article: Locator, isMobile: boolean) {
  if (isMobile) {
    await longPress(page, article.locator(".message-body").first());
    const sheet = page.getByRole("dialog", { name: "Message actions" });
    await sheet.getByRole("button", { name: "Reply", exact: true }).tap();
    await expect(sheet).toHaveCount(0);
  } else {
    await article.hover();
    await article
      .getByRole("group", { name: "Message actions" })
      .getByRole("button", { name: "Reply", exact: true })
      .click();
  }
}

/** Signs in and notes the bodies of the messages the app posts. */
async function signIn(page: Page): Promise<unknown[]> {
  const posted: unknown[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().endsWith(`/channels/${general}/messages`)) {
      posted.push(request.postDataJSON());
    }
  });
  await signInToWorld(page);
  return posted;
}

test("a reply tags the author of what it answers, and shows it above itself", async ({
  page,
  isMobile,
}) => {
  const posted = await signIn(page);
  await openGeneral(page);
  await reply(page, row(page, answeredText), isMobile);
  const bar = page.locator("form").getByText(/^Replying to/);
  await expect(bar).toContainText("Bob With A Rather Long Display Name");
  await expect(page.getByRole("button", { name: /^Tag Bob/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(box(page)).toBeFocused();
  await box(page).fill("Ten works for me");
  await submit(page);
  await expect(bar).toHaveCount(0);
  expect(posted).toEqual([
    {
      content: "Ten works for me",
      attachments: [],
      replyTo: answered,
      tagReplied: true,
      mayHold: true,
    },
  ]);

  const sent = row(page, "Ten works for me");
  const line = sent.getByRole("button", { name: /^Replying to/ });
  await expect(line).toContainText(answeredText);
  await expect(line.getByText(bobName)).toHaveText(/^@/);
  // At its start: the actions of the row under the pointer rise over the line's end.
  await line.click({ position: { x: 4, y: 8 } });
  await expect(page.locator(`article[data-message-id="${answered}"]`)).toHaveClass(/motion-flash/);
});

test("a reply may leave its author untagged, and Escape lets it go", async ({ page, isMobile }) => {
  const posted = await signIn(page);
  await openGeneral(page);
  await reply(page, row(page, answeredText), isMobile);
  const bar = page.locator("form").getByText(/^Replying to/);
  await expect(bar).toBeVisible();
  await box(page).focus();
  await page.keyboard.press("Escape");
  await expect(bar).toHaveCount(0);

  await reply(page, row(page, answeredText), isMobile);
  await page.getByRole("button", { name: /^Tag Bob/ }).click();
  await expect(page.getByRole("button", { name: /^Tag Bob/ })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await box(page).fill("No rush");
  await submit(page);
  await expect(row(page, "No rush")).toBeVisible();
  expect(posted).toEqual([
    { content: "No rush", attachments: [], replyTo: answered, tagReplied: false, mayHold: true },
  ]);
  await expect(
    row(page, "No rush")
      .getByRole("button", { name: /^Replying to/ })
      .getByText(bobName),
  ).not.toHaveText(/^@/);
});

test("a reply to one's own message offers no tag, from the message's menu too", async ({
  page,
  isMobile,
}) => {
  await signIn(page);
  await openGeneral(page);
  if (isMobile) {
    await reply(page, row(page, ownText), isMobile);
  } else {
    await row(page, ownText).locator(".message-body").click({ button: "right" });
    await page.getByRole("menuitem", { name: "Reply", exact: true }).click();
  }
  await expect(page.locator("form").getByText(/^Replying to/)).toBeVisible();
  await expect(box(page)).toBeFocused();
  await expect(page.getByRole("button", { name: /^Tag / })).toHaveCount(0);
});

test("a reply whose original is deleted says so", async ({ page }) => {
  const publish = await signInToWorld(page);
  await openGeneral(page);
  const replyId = messageId(970);
  publish({
    serverEvent: "message",
    type: "create",
    ...message(970, bob, "Chairs would be great", 0, {
      replyTo: messageId(206),
      mentions: { users: [me], roles: [], everyone: false },
    }),
  });
  const line = page.locator(`article[data-message-id="${replyId}"]`);
  await expect(line.getByRole("button", { name: /^Replying to/ })).toContainText(ownText);
  publish({ serverEvent: "message", type: "delete", id: messageId(206) });
  await expect(line.getByText("Original message was deleted")).toBeVisible();
});
