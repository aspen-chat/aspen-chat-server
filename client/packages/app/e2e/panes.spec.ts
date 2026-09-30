import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Resizable panes, against the stubbed world: the channel list, member list, and thread panel
 * each have an edge the reader drags or moves from the keyboard, and this device keeps the
 * width.
 */

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows one pane at a time");
  await signInToWorld(page);
  await page.getByText("general", { exact: true }).click();
});

const edge = (page: Page, pane: string) =>
  page.getByRole("separator", { name: `Resize the ${pane}` });

async function widthOf(page: Page, pane: string): Promise<number> {
  return edge(page, pane).evaluate((e) => e.parentElement?.getBoundingClientRect().width ?? 0);
}

test("dragging the channel list's edge widens it, and the width outlasts a reload", async ({
  page,
}) => {
  const before = await widthOf(page, "channel list");
  const box = await edge(page, "channel list").boundingBox();
  if (box === null) {
    throw new Error("the edge is not shown");
  }
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 60, y, { steps: 5 });
  await page.mouse.up();
  await expect.poll(() => widthOf(page, "channel list")).toBeCloseTo(before + 60, -1);
  await page.reload();
  await expect.poll(() => widthOf(page, "channel list")).toBeCloseTo(before + 60, -1);
  // A double-click puts it back.
  await edge(page, "channel list").dblclick();
  await expect.poll(() => widthOf(page, "channel list")).toBeCloseTo(before, -1);
});

test("the member list's edge moves from the keyboard, within its bounds", async ({ page }) => {
  const separator = edge(page, "member list");
  const before = Number(await separator.getAttribute("aria-valuenow"));
  await separator.focus();
  // The member list is at the end, so its edge moves outward to the left.
  await page.keyboard.press("ArrowLeft");
  await expect(separator).toHaveAttribute("aria-valuenow", String(before + 16));
  await page.keyboard.press("End");
  await expect(separator).toHaveAttribute(
    "aria-valuenow",
    (await separator.getAttribute("aria-valuemax")) ?? "",
  );
  await page.keyboard.press("Enter");
  await expect(separator).toHaveAttribute("aria-valuenow", String(before));
});
