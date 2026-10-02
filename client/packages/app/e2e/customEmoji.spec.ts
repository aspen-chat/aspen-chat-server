import { expect, test, type Page } from "@playwright/test";
import { customEmojiId, customEmojiName, ownText, signInToWorld } from "./world";

/** A one-pixel PNG, as a file to upload. */
const PIXEL_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  "base64",
);

async function openGeneral(page: Page) {
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
}

test("a custom emoji in a message renders as its picture, named for a screen reader", async ({
  page,
}) => {
  await openGeneral(page);
  const morning = page.locator("article").filter({ hasText: "Morning all!" }).last();
  const glyph = morning.getByRole("img", { name: `:${customEmojiName}:` });
  await expect(glyph).toBeVisible();
  await expect(glyph).toHaveAttribute("src", /^data:image\/png/);
  // The reference itself never shows as text.
  await expect(morning).not.toContainText("<:");
});

test("a colon completes the community's emoji and unicode emoji, and sends the reference", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "Enter writes a new line on a phone; the completion is the same");
  await openGeneral(page);
  const sent: unknown[] = [];
  await page.route(/\/api\/v1\/channels\/[^/]+\/messages$/, async (route) => {
    if (route.request().method() === "POST") {
      sent.push(route.request().postDataJSON());
    }
    await route.fallback();
  });
  const box = page.getByRole("textbox", { name: "Message" });
  await box.pressSequentially("hooray :par");
  const list = page.getByRole("listbox", { name: "Emoji to insert" });
  await expect(list).toBeVisible();
  // The community's own first, then unicode emoji by name.
  await expect(list.getByRole("option").first()).toHaveText(new RegExp(`:${customEmojiName}:`));
  await expect(list.getByRole("option", { name: /party/ }).nth(1)).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(box).toHaveValue(`hooray :${customEmojiName}: `);
  await box.press("Enter");
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0]).toMatchObject({ content: `hooray <:${customEmojiId}>` });
});

test("a custom emoji reacts from the picker, and its chip shows the picture", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a phone reaches the picker by a long press, which reactions.spec drives");
  await openGeneral(page);
  // The caller's own message, which has no reactions yet, so the new one is its first chip.
  const reacted = page.locator("article").filter({ hasText: ownText }).last();
  await reacted.hover();
  await reacted
    .getByRole("group", { name: "Message actions" })
    .getByRole("button", { name: "Add a reaction" })
    .click();
  const picker = page.locator(".emoji-picker");
  await expect(picker).toBeVisible();
  const custom = picker.getByRole("button", { name: new RegExp(customEmojiName) });
  await expect(custom).toBeVisible();
  await custom.click();
  await expect(picker).toHaveCount(0);
  const chips = reacted.getByRole("list", { name: "Reactions" });
  const chip = chips.getByRole("button", { name: `Remove your :${customEmojiName}:` });
  await expect(chip).toBeVisible();
  await expect(chip.getByRole("img", { name: `:${customEmojiName}:` })).toBeVisible();
});

test("the Emoji tab adds, renames, and removes an emoji", async ({ page }) => {
  await signInToWorld(page);
  await page.getByRole("button", { name: "Community settings" }).click();
  const dialog = page.getByRole("dialog", { name: /settings/ });
  await dialog.getByRole("tab", { name: "Emoji" }).click();
  await expect(dialog.getByText(`:${customEmojiName}:`)).toBeVisible();
  // Adding: a picture, which fills the name in from the file's, and the name.
  await dialog.getByLabel("Picture").setInputFiles({
    name: "blobcat.png",
    mimeType: "image/png",
    buffer: PIXEL_PNG,
  });
  const name = dialog.getByRole("textbox", { name: "Name" });
  await expect(name).toHaveValue("blobcat");
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  // The row is found by its picture, whose name stays while the name is being edited.
  const rowOf = (name: string) =>
    dialog.getByRole("listitem").filter({ has: page.getByRole("img", { name: `:${name}:` }) });
  const row = rowOf("blobcat");
  await expect(row).toBeVisible();
  await expect(row).toContainText(":blobcat:");
  // Renaming, in place.
  await row.getByRole("button", { name: "Rename" }).click();
  await row.getByRole("textbox", { name: "Name" }).fill("blobcat2");
  await row.getByRole("button", { name: "Save" }).click();
  const renamed = rowOf("blobcat2");
  await expect(renamed).toContainText(":blobcat2:");
  // Removing asks once.
  await renamed.getByRole("button", { name: "Remove" }).click();
  await expect(renamed).toContainText("every reaction");
  await renamed.getByRole("button", { name: "Remove", exact: true }).last().click();
  await expect(dialog.getByRole("listitem").filter({ hasText: ":blobcat2:" })).toHaveCount(0);
});
