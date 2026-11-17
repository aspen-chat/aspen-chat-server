import { expect, test, type Page } from "@playwright/test";
import { general, me, signInToWorld, submit } from "./world";
import { message, messageId } from "./world/fixtures";

/**
 * Emoticons typed in the message box become emoji (`useEmoticons`), against the stubbed world:
 * as the space after one is typed, or as the message is sent; Backspace just after puts the
 * emoticon back without deleting anything; and the setting, on until turned off, stops it all.
 */

const box = (page: Page) => page.getByRole("textbox", { name: "Message" });

/** Signs in, with the account's preferences as `values`, and notes the messages posted. */
async function signIn(
  page: Page,
  values: Record<string, unknown> = {},
  opened?: (publish: Awaited<ReturnType<typeof signInToWorld>>) => void,
): Promise<unknown[]> {
  const posted: unknown[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().endsWith(`/channels/${general}/messages`)) {
      posted.push((request.postDataJSON() as { content: unknown }).content);
    }
  });
  const publish = await signInToWorld(page, async (page) => {
    await page.route(/\/api\/v1\/users\/(@|%40)me\/preferences$/, async (route) => {
      if (route.request().method() !== "GET") {
        await route.fallback();
        return;
      }
      await route.fulfill({ json: { values, updatedAt: new Date().toISOString() } });
    });
  });
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText("general", { exact: true }).first().click();
  opened?.(publish);
  await box(page).click();
  return posted;
}

test("an emoticon becomes its emoji as the space after it is typed", async ({ page }) => {
  const posted = await signIn(page);
  await page.keyboard.type("hi :) there <3!");
  await expect(box(page)).toHaveValue("hi 🙂 there ❤️!");
  // Only what stands on its own is a face.
  await page.keyboard.type(" http://x a:p `a :D` ");
  await expect(box(page)).toHaveValue("hi 🙂 there ❤️! http://x a:p `a :D` ");
  await submit(page);
  await expect.poll(() => posted).toEqual(["hi 🙂 there ❤️! http://x a:p `a :D`"]);
});

test("Backspace just after puts the emoticon back, deleting nothing", async ({ page }) => {
  const posted = await signIn(page);
  await page.keyboard.type("ok :D ");
  await expect(box(page)).toHaveValue("ok 😃 ");
  await page.keyboard.press("Backspace");
  await expect(box(page)).toHaveValue("ok :D ");
  await expect(box(page)).toHaveJSProperty("selectionStart", "ok :D ".length);
  // The one put back stays as typed, even with the space after it typed again; another still
  // converts.
  await page.keyboard.press("Backspace");
  await expect(box(page)).toHaveValue("ok :D");
  await page.keyboard.type(" ");
  await expect(box(page)).toHaveValue("ok :D ");
  await page.keyboard.type("y :D ");
  await expect(box(page)).toHaveValue("ok :D y 😃 ");
  // Backspace after anything else deletes as ever.
  await page.keyboard.type("z");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Backspace");
  await expect(box(page)).toHaveValue("ok :D y 😃");
  await page.keyboard.type(" :(");
  await submit(page);
  // One left at the end becomes its emoji as it is sent.
  await expect.poll(() => posted).toEqual(["ok :D y 😃 🙁"]);
});

test("a caret moved before Backspace keeps the emoji", async ({ page }) => {
  await signIn(page);
  await page.keyboard.type("a :) ");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Backspace");
  await expect(box(page)).toHaveValue("a 🙂");
});

test("with the setting off, emoticons stay as typed", async ({ page }) => {
  const posted = await signIn(page, { "composing.emoticonEmoji": false });
  await page.keyboard.type("hi :) ");
  await expect(box(page)).toHaveValue("hi :) ");
  await page.keyboard.type(":(");
  await submit(page);
  await expect.poll(() => posted).toEqual(["hi :) :("]);
});

test("the setting is on until turned off, and kept with the account", async ({ page }) => {
  await signIn(page);
  // On a phone the channel stands in the rail's place.
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  const checkbox = settings.getByRole("checkbox", { name: "Turn emoticons into emoji" });
  await expect(checkbox).toBeChecked();
  const saved = page.waitForRequest(
    (r) => r.method() === "PATCH" && /\/users\/(@|%40)me\/preferences$/.test(r.url()),
  );
  await settings.getByText("Turn emoticons into emoji", { exact: true }).click();
  expect((await saved).postDataJSON()).toEqual({ "composing.emoticonEmoji": false });
  await expect(checkbox).not.toBeChecked();
});

test("an edit keeps the message's own emoticons as written", async ({ page, isMobile }) => {
  test.skip(isMobile, "a phone reaches Edit by a long press, which mobile.spec drives");
  await signIn(page, {}, (publish) => {
    publish({ serverEvent: "message", type: "create", ...message(980, me, "see you :)", 0) });
  });
  const edited = page.waitForRequest(
    (r) => r.method() === "PATCH" && r.url().endsWith(`/messages/${messageId(980)}`),
  );
  const own = page.locator(`article[data-message-id="${messageId(980)}"]`);
  await own.getByText("see you :)").click({ button: "right" });
  await page
    .getByRole("menu", { name: "Message actions" })
    .getByRole("menuitem", { name: "Edit message" })
    .click();
  const editor = page.getByRole("textbox", { name: "Edit message" });
  // What is typed converts; the emoticon the message ends with, sent as written, stays.
  await editor.press("Home");
  await page.keyboard.type(":D ");
  await expect(editor).toHaveValue("😃 see you :)");
  await editor.press("Enter");
  expect((await edited).postDataJSON()).toEqual({ content: "😃 see you :)" });
});
