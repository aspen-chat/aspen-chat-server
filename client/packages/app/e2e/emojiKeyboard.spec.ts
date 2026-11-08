import { expect, test, type Page } from "@playwright/test";
import { customEmojiName, ownText, signInToWorld } from "./world";

/**
 * The emoji keyboard (`EmojiKeyboard`): the picker docked below the message box by the smiley
 * left of Send, and below a message being edited, offering unicode emoji and the community's
 * own, staying open across picks.
 */

/** The grinning face, as the picker's buttons name it and as a message writes it. */
const GRINNING_FACE = "1f600";
const GRINNING_FACE_GLYPH = "\u{1F600}";

async function openGeneral(page: Page) {
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
}

test("the smiley docks the keyboard below the box, and picks go in at the caret", async ({
  page,
}) => {
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("hello world");
  const smiley = page.getByRole("button", { name: "Insert an emoji" });
  await expect(smiley).toHaveAttribute("aria-expanded", "false");
  await smiley.click();
  await expect(smiley).toHaveAttribute("aria-expanded", "true");
  const keyboard = page.getByRole("group", { name: "Emoji keyboard" });
  await expect(keyboard.locator(".emoji-picker")).toBeVisible();
  // Docked: below the box, not a popover or a sheet, edge to edge across the conversation, with
  // its emoji running edge to edge too, at most one emoji's room short.
  const boxBounds = await box.boundingBox();
  const keyboardBounds = await keyboard.boundingBox();
  const formBounds = await page.locator("form", { has: box }).boundingBox();
  const pickerBounds = await keyboard.locator(".emoji-picker").boundingBox();
  expect(keyboardBounds?.y ?? 0).toBeGreaterThan((boxBounds?.y ?? 0) + (boxBounds?.height ?? 0));
  expect(keyboardBounds?.x).toBe(formBounds?.x);
  expect(keyboardBounds?.width).toBe(formBounds?.width);
  expect(pickerBounds?.width ?? 0).toBeGreaterThan((keyboardBounds?.width ?? 0) - 40);
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // The community's own emoji are a section of the keyboard, written as the box shows them.
  const custom = keyboard.getByRole("button", { name: new RegExp(customEmojiName) });
  await custom.scrollIntoViewIfNeeded();
  await custom.click();
  await expect(box).toHaveValue(`hello world:${customEmojiName}:`);
  // The keyboard stays for the next pick, which goes in where the caret is.
  await expect(keyboard).toBeVisible();
  const grin = keyboard.locator(`button[data-unified="${GRINNING_FACE}"]`);
  await grin.scrollIntoViewIfNeeded();
  await grin.click();
  await expect(box).toHaveValue(`hello world:${customEmojiName}:${GRINNING_FACE_GLYPH}`);
  await expect(keyboard).toBeVisible();

  // Escape in the box puts it away, and the smiley opens it again.
  await box.press("Escape");
  await expect(keyboard).toHaveCount(0);
  await expect(smiley).toHaveAttribute("aria-expanded", "false");
  await smiley.click();
  await expect(keyboard).toBeVisible();
  await smiley.click();
  await expect(keyboard).toHaveCount(0);
});

test("the message editor has the keyboard too, below the message", async ({ page, isMobile }) => {
  test.skip(isMobile, "a phone reaches Edit by a long press, which mobile.spec drives");
  await openGeneral(page);
  const own = page.locator("article").filter({ hasText: ownText }).last();
  await own.getByText(ownText).click({ button: "right" });
  await page
    .getByRole("menu", { name: "Message actions" })
    .getByRole("menuitem", { name: "Edit message" })
    .click();
  const editor = page.getByRole("textbox", { name: "Edit message" });
  await expect(editor).toBeVisible();
  await editor.press("End");
  await own.getByRole("button", { name: "Insert an emoji" }).click();
  const keyboard = own.getByRole("group", { name: "Emoji keyboard" });
  await expect(keyboard.locator(".emoji-picker")).toBeVisible();
  const grin = keyboard.locator(`button[data-unified="${GRINNING_FACE}"]`);
  await grin.scrollIntoViewIfNeeded();
  await grin.click();
  await expect(editor).toHaveValue(`${ownText}${GRINNING_FACE_GLYPH}`);
  // Escape puts the keyboard away first; a second Escape ends the edit.
  await editor.press("Escape");
  await expect(keyboard).toHaveCount(0);
  await expect(editor).toBeVisible();
  await editor.press("Escape");
  await expect(editor).toHaveCount(0);
});
