import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * A channel's mana settings, as the organiser of the stubbed world in `world.ts` sees them: they
 * may manage channels, and the deployment's own mana is what each field shows until changed.
 */

test("a channel's mana fields span the dialog, wide enough for what they say they follow", async ({
  page,
}) => {
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByRole("row", { name: /general/ })
    .getByRole("button", { name: /Options for general/ })
    .click();
  await page.getByRole("menuitem", { name: "Channel mana" }).click();
  const dialog = page.getByRole("dialog", { name: "Mana of #general" });
  const formWidth = await dialog.locator("form").evaluate((form) => form.clientWidth);
  for (const [name, placeholder] of [
    ["Mana earned back a second", "This server's: 200"],
    ["Mana it holds", "This server's: 12000"],
  ] as const) {
    const field = dialog.getByRole("textbox", { name });
    await expect(field).toHaveAttribute("placeholder", placeholder);
    const room = await field.evaluate((input: HTMLInputElement) => {
      const style = getComputedStyle(input);
      const context = document.createElement("canvas").getContext("2d");
      if (context === null) {
        throw new Error("no canvas");
      }
      context.font = style.font;
      const inside =
        input.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      return {
        width: input.offsetWidth,
        inside,
        text: context.measureText(input.placeholder).width,
      };
    });
    expect(room.width).toBe(formWidth);
    expect(room.text).toBeLessThanOrEqual(room.inside);
  }
});
