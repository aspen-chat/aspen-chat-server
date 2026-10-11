import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Resizable panes, against the stubbed world: the channel list, member list, and thread panel
 * each have an edge the reader drags or moves from the keyboard, and this device keeps the
 * width; the bars at their heads are one height, so the rules under them meet.
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

test("the bars at the head of the channel list, the channel, and a thread are one height", async ({
  page,
}) => {
  await page
    .getByRole("link", { name: /^View thread/ })
    .first()
    .click();
  const heights = () =>
    Promise.all(
      [
        page.getByRole("heading", { level: 1, name: "Family" }).locator(".."),
        page.locator("main > header"),
        page.getByRole("complementary", { name: "Thread" }).locator("header"),
      ].map(async (bar) => (await bar.boundingBox())?.height),
    );
  await expect.poll(heights).toEqual([53, 53, 53]);
  await page.goto("/dms");
  await page.getByRole("navigation", { name: "Direct messages" }).getByRole("link").first().click();
  await expect
    .poll(async () => [
      (await page.getByRole("heading", { level: 1 }).locator("..").boundingBox())?.height,
      (await page.locator("main > header").boundingBox())?.height,
    ])
    .toEqual([53, 53]);
});
