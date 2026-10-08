import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Choosing one emoji, or none, for a poll's option (`EmojiChoiceButton`): a popover where there
 * is a pointer, a sheet from the bottom on a touch screen.
 */

test("an option's emoji is chosen from the picker, and cleared from it", async ({
  page,
  isMobile,
}) => {
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  // On a narrow screen the poll is behind the box's + button.
  const more = page.getByRole("button", { name: "Add a file or a poll" });
  if (await more.isVisible()) {
    await more.click();
    await page.getByRole("menuitem", { name: "Create a new poll" }).click();
  } else {
    await page.getByRole("button", { name: "Create a new poll" }).click();
  }
  const form = page.getByRole("dialog", { name: "New poll" });
  await form.getByRole("button", { name: "Choose an emoji for option 1" }).click();
  const picker = page.getByRole("dialog", { name: "Choose an emoji for option 1" });
  await expect(picker.locator(".emoji-picker")).toBeVisible();
  if (isMobile) {
    // A sheet across the bottom of the screen.
    const box = await picker.boundingBox();
    const viewport = page.viewportSize();
    expect(box?.width ?? 0).toBeCloseTo(viewport?.width ?? 0, 0);
  }
  const first = picker.locator(".epr-emoji-category-content > button").first();
  await expect(first).toBeVisible();
  const emoji = (await first.getAttribute("data-unified")) ?? "";
  await first.click();
  await expect(picker).toHaveCount(0);
  const chosen = form.getByRole("button", { name: "Change the emoji for option 1" });
  await expect(chosen).toBeVisible();
  expect(emoji).not.toBe("");
  await chosen.click();
  await page.getByRole("button", { name: "Remove emoji" }).click();
  await expect(form.getByRole("button", { name: "Choose an emoji for option 1" })).toBeVisible();
});
