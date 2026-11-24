import { expect, test, type Page } from "@playwright/test";
import { longPress, signInToWorld, type Publish } from "./world";
import { general, generalMessages, me, message, messageId, users } from "./world/fixtures";

/**
 * Taking a file off a sent message, against the stubbed world: offered in the attachment's own
 * menu (a right click on a computer, a long press on a phone), never by a button on the file, and
 * done only once the reader has confirmed it.
 */

const fileId = "0190f0a0-0000-7000-8002-000000000041";
const ownId = messageId(160);

const file = {
  id: fileId,
  fileName: "minutes.zip",
  mimeType: "application/zip",
  downloadUrl: "https://media.test/world/minutes.zip",
};

/**
 * The newest message of #general, the caller's own, carrying `file`; it loses the file once the
 * server has been asked to remove it, announced by `publish` as the server would. Returns the
 * removals asked for, by attachment id.
 */
async function withOwnFile(page: Page, publish: () => Publish | undefined): Promise<string[]> {
  const removed: string[] = [];
  const own = () =>
    message(160, me, "The minutes", 1, { attachments: removed.includes(fileId) ? [] : [fileId] });
  await page.route(/\/api\/v1\/attachments\/[^/?]+$/, (route) =>
    new URL(route.request().url()).pathname.endsWith(fileId)
      ? route.fulfill({ json: file })
      : route.fallback(),
  );
  await page.route(new RegExp(`/api/v1/messages/${ownId}(/.*)?(\\?.*)?$`), (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.method() === "DELETE" && path.endsWith(`/attachments/${fileId}`)) {
      removed.push(fileId);
      publish()?.({ serverEvent: "message", type: "update", id: ownId, attachments: [] });
      return route.fulfill({ status: 204, body: "" });
    }
    if (request.method() === "GET" && path.endsWith(ownId)) {
      return route.fulfill({ json: own() });
    }
    return route.fallback();
  });
  await page.route(new RegExp(`/api/v1/channels/${general}/messages(\\?.*)?$`), (route) => {
    const url = new URL(route.request().url());
    if (url.searchParams.has("before") || url.searchParams.has("after")) {
      return route.fulfill({ json: { data: [], included: { users: [] } } });
    }
    return route.fulfill({
      json: { data: [own(), ...generalMessages], included: { users } },
    });
  });
  return removed;
}

const ownMessage = (page: Page) =>
  page.locator("article").filter({ hasText: "The minutes" }).last();
const fileLink = (page: Page) => ownMessage(page).getByRole("link", { name: "minutes.zip" });

test("a right-clicked file is removed from the menu once confirmed", async ({ page, isMobile }) => {
  test.skip(isMobile, "A touch screen offers it by a long press.");
  let removed: string[] = [];
  // Asked for only once the removal is, long after signing in has returned it.
  const publish: Publish = await signInToWorld(page, async (p) => {
    removed = await withOwnFile(p, () => publish);
  });
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(fileLink(page)).toBeVisible();
  await ownMessage(page).hover();
  await expect(ownMessage(page).getByRole("button", { name: /Remove/ })).toHaveCount(0);

  const menu = page.getByRole("menu", { name: "Message actions" });
  const dialog = page.getByRole("alertdialog", { name: "Remove minutes.zip?" });
  // Dismissed, the confirmation removes nothing.
  await fileLink(page).click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Remove attachment" }).click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  expect(removed).toEqual([]);

  await fileLink(page).click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Remove attachment" }).click();
  await dialog.getByRole("button", { name: "Remove" }).click();
  await expect(dialog).toBeHidden();
  expect(removed).toEqual([fileId]);
  await expect(fileLink(page)).toHaveCount(0);
});

test("a link's picture offers no removal in the menu", async ({ page, isMobile }) => {
  test.skip(isMobile, "A touch screen offers it by a long press.");
  await signInToWorld(page, async (p) => {
    await withOwnFile(p, () => undefined);
  });
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  // Bob's message's link preview, a picture that is no attachment of the message.
  const picture = page.locator("article").locator("[data-media-picture]").first();
  await picture.scrollIntoViewIfNeeded();
  await picture.click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Message actions" });
  await expect(menu.getByRole("menuitem", { name: "View image" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Remove attachment" })).toHaveCount(0);
});

test("a long-pressed file is removed from its own menu once confirmed", async ({
  page,
  isMobile,
  browserName,
}) => {
  test.skip(!isMobile, "A computer offers it by a right click.");
  test.skip(browserName !== "chromium", "only Chromium takes synthetic touch input");
  let removed: string[] = [];
  // Asked for only once the removal is, long after signing in has returned it.
  const publish: Publish = await signInToWorld(page, async (p) => {
    removed = await withOwnFile(p, () => publish);
  });
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .tap();
  await expect(fileLink(page)).toBeVisible();
  await expect(ownMessage(page).getByRole("button", { name: /Remove/ })).toHaveCount(0);

  await longPress(page, fileLink(page));
  const menu = page.getByRole("menu", { name: "Attachment actions" });
  await expect(menu).toBeVisible();
  // The item's long press is its own; the message's sheet stays shut.
  await expect(page.getByRole("dialog", { name: "Message actions" })).toHaveCount(0);
  await expect(menu.getByRole("menuitem", { name: "Save file" })).toBeVisible();
  await menu.getByRole("menuitem", { name: "Remove attachment" }).tap();
  const dialog = page.getByRole("alertdialog", { name: "Remove minutes.zip?" });
  await expect(dialog).toBeVisible();
  expect(removed).toEqual([]);
  await dialog.getByRole("button", { name: "Remove" }).tap();
  await expect(dialog).toBeHidden();
  expect(removed).toEqual([fileId]);
  await expect(fileLink(page)).toHaveCount(0);
});
