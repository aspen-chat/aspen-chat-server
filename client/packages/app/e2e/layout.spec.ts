import { expect, test, type Locator, type Page } from "@playwright/test";
import { settleAnimations, signInToWorld } from "./world";

/**
 * The Layout tab of the community's settings, as the organiser of the stubbed world in
 * `world.ts` sees it: they may manage channels and categories.
 */

async function openLayout(page: Page): Promise<Locator> {
  await signInToWorld(page);
  await page.getByRole("button", { name: "Community settings" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("tab", { name: "Layout" }).click();
  return dialog.getByRole("grid", { name: "Channels and categories" });
}

/** Each row's name, which its buttons' labels follow. */
const names = (grid: Locator) =>
  grid
    .getByRole("row")
    .evaluateAll((rows) => rows.map((row) => row.querySelector("span")?.textContent ?? ""));

/** Picks a row up from its drag handle, which the arrow keys reach from the row. */
async function pickUp(grid: Locator, name: string): Promise<void> {
  await grid.getByRole("row", { name }).focus();
  await grid.page().keyboard.press("ArrowRight");
  await grid.page().keyboard.press("Enter");
}

/** Moves a keyboard drag down the places the row may go until it reaches the one named. */
async function moveTo(grid: Locator, place: string): Promise<void> {
  const page = grid.page();
  const focused = () => page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
  for (let step = 0; step < 20 && (await focused()) !== place; step += 1) {
    await page.keyboard.press("ArrowDown");
  }
  expect(await focused()).toBe(place);
}

test("the layout lists the top level, then each category with its channels", async ({ page }) => {
  const grid = await openLayout(page);
  expect(await names(grid)).toEqual([
    "No category",
    "general",
    "Lounge",
    "Planning",
    "roadmap",
    "ideas",
    "Archive",
  ]);
});

test("a category dragged by keyboard moves with its channels", async ({ page }) => {
  const grid = await openLayout(page);
  await pickUp(grid, "Planning");
  // The only place Planning may go is the end of the list, after Archive.
  await moveTo(grid, "Insert after Archive");
  await page.keyboard.press("Enter");
  await settleAnimations(page);
  expect(await names(grid)).toEqual([
    "No category",
    "general",
    "Lounge",
    "Archive",
    "Planning",
    "roadmap",
    "ideas",
  ]);
});

test("a channel dragged by keyboard onto a category joins it", async ({ page }) => {
  const grid = await openLayout(page);
  await pickUp(grid, "general");
  await moveTo(grid, "Drop on Archive");
  await page.keyboard.press("Enter");
  await settleAnimations(page);
  expect(await names(grid)).toEqual([
    "No category",
    "Lounge",
    "Planning",
    "roadmap",
    "ideas",
    "Archive",
    "general",
  ]);
  // The channel list beside the dialog follows.
  await expect(page.getByRole("grid", { name: "Archive channels" }).getByRole("row")).toContainText(
    ["general"],
  );
});

test("categories are made, renamed, and deleted from the layout", async ({ page }) => {
  const grid = await openLayout(page);
  const settings = page.getByRole("dialog").first();
  await settings.getByRole("button", { name: "New category" }).click();
  await page.getByRole("textbox", { name: "Category name" }).fill("Games");
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await expect(grid.getByRole("row").last()).toContainText("Games");
  await grid.getByRole("button", { name: "Rename Games" }).click();
  await page.getByRole("textbox", { name: "Category name" }).fill("Play");
  await page.getByRole("button", { name: "Rename", exact: true }).click();
  await expect(grid.getByRole("row").last()).toContainText("Play");
  await grid.getByRole("button", { name: "Delete Play" }).click();
  await page.getByRole("button", { name: "Delete category" }).click();
  await expect(grid.getByRole("row")).toHaveCount(7);
});
