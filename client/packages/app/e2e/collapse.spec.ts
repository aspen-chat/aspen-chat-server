import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Folding a category, against the stubbed world in `world.ts`, where Planning holds #roadmap,
 * unread, and #ideas, read, and Archive holds nothing.
 */

const planning = (page: Page) => page.getByRole("button", { name: "Planning", exact: true });
const planningList = (page: Page) => page.getByRole("grid", { name: "Planning channels" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("a folded category keeps only what needs attention in view", async ({ page }) => {
  await expect(planning(page)).toHaveAttribute("aria-expanded", "true");
  await expect(planningList(page).getByText("ideas", { exact: true })).toBeVisible();
  await planning(page).click();
  await expect(planning(page)).toHaveAttribute("aria-expanded", "false");
  await expect(planningList(page).getByText("ideas", { exact: true })).toHaveCount(0);
  // Unread, so still in view.
  await expect(planningList(page).getByText("roadmap, unread")).toBeAttached();

  // Muted, it needs no attention, and goes too.
  await planningList(page).getByText("roadmap", { exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "For 1 hour" }).click();
  await expect(planningList(page).getByText("roadmap", { exact: true })).toHaveCount(0);
  // Nothing left in view, but the category is not empty, and must never say it is.
  await expect(planningList(page).getByText("No channels yet; drop one here")).toHaveCount(0);

  await planning(page).click();
  await expect(planning(page)).toHaveAttribute("aria-expanded", "true");
  await expect(planningList(page).getByText("ideas", { exact: true })).toBeVisible();
  await expect(planningList(page).getByText("roadmap, muted")).toBeAttached();
});

test("the channel being read stays in view under its folded category", async ({
  page,
  isMobile,
}) => {
  // A phone shows the list or a channel, never both, so no channel is being read beside it.
  test.skip(isMobile, "the list and a channel share the screen only on a wide one");
  await planningList(page).getByText("ideas", { exact: true }).click();
  await expect(page.getByRole("heading", { name: "ideas" })).toBeVisible();
  await planning(page).click();
  await expect(planning(page)).toHaveAttribute("aria-expanded", "false");
  await expect(planningList(page).getByText("ideas", { exact: true })).toBeVisible();
});

test("a folded category with no channels at all does not say it is empty", async ({ page }) => {
  const archive = page.getByRole("button", { name: "Archive", exact: true });
  await expect(
    page
      .getByRole("grid", { name: "Archive channels" })
      .getByText("No channels yet; drop one here"),
  ).toBeVisible();
  await archive.click();
  await expect(archive).toHaveAttribute("aria-expanded", "false");
  await expect(
    page
      .getByRole("grid", { name: "Archive channels" })
      .getByText("No channels yet; drop one here"),
  ).toHaveCount(0);
});
