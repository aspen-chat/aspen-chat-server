import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { community, signInToWorld, settleAnimations } from "./world";

/**
 * A folder on the community rail, against the stubbed world in `world.ts`, whose account
 * preferences here hold one folder around its community. How drops arrange the rail is
 * `railOrder.test.ts`'s; this checks the folder as the reader meets it.
 */

async function withFolder(page: Page) {
  await page.route(/\/api\/v1\/users\/(@|%40)me\/preferences$/, async (route) => {
    if (route.request().method() !== "GET") {
      await route.fallback();
      return;
    }
    await route.fulfill({
      json: {
        values: {
          "rail.order": ["folder:kin"],
          "rail.folders": [
            { id: "kin", name: "Kin", color: "sky", open: false, members: [community] },
          ],
        },
        updatedAt: new Date().toISOString(),
      },
    });
  });
}

async function axeProblems(page: Page, popover = false): Promise<string[]> {
  await settleAnimations(page);
  let builder = new AxeBuilder({ page });
  if (popover) {
    builder = builder.disableRules(["region"]);
  }
  const audit = await builder.analyze();
  return audit.violations.flatMap((v) => v.nodes.map((n) => `${v.id} at ${n.target.join(" ")}`));
}

test("a folder opens in place, offers its menu, and passes axe", async ({ page }) => {
  await signInToWorld(page, withFolder);
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  const rail = page.getByRole("grid", { name: "Communities" });
  const folder = rail.getByRole("row", { name: /^Kin folder, 1 community/ });
  await expect(folder).toBeVisible();
  await expect(rail.getByRole("row", { name: /Family/ })).toHaveCount(0);
  expect.soft(await axeProblems(page)).toEqual([]);

  await folder.click();
  await expect(rail.getByRole("row", { name: /^Kin folder, open, 1 community/ })).toBeVisible();
  await expect(rail.getByRole("row", { name: /Family/ })).toBeVisible();
  expect.soft(await axeProblems(page)).toEqual([]);

  await rail
    .getByRole("row", { name: /^Kin folder/ })
    .locator("div")
    .first()
    .click({
      button: "right",
    });
  const menu = page.getByRole("menu", { name: "Options for Kin folder" });
  await expect(menu.getByRole("menuitem", { name: "Rename" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Ungroup" })).toBeVisible();
  expect.soft(await axeProblems(page, true)).toEqual([]);
  await menu.getByRole("menuitem", { name: "Ungroup" }).click();
  await expect(rail.getByRole("row", { name: /folder/ })).toHaveCount(0);
  await expect(rail.getByRole("row", { name: /Family/ })).toBeVisible();
});
