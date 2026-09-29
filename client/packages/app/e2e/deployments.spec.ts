import { expect, test, type Page } from "@playwright/test";
import { foreignCommunity, foreignDomain, stubForeignDeployment } from "./foreignWorld";
import { signInToWorld } from "./world";

/**
 * Other deployments beside the home, against the stubbed world in `world.ts` and a stubbed
 * `beta.example` (`foreignWorld.ts`): the rail and the DM list mix them, each foreign entry
 * marked with its domain, and their routes live under `/at/{domain}`.
 */

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test("a deployment the home lists joins the rail and the DM list", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  const entry = rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` });
  await expect(entry).toBeVisible();
  await expect(rail(page).getByRole("row", { name: /^Family/ })).toBeVisible();
  await entry.click();
  await expect(page).toHaveURL(new RegExp(`/at/beta\\.example/communities/${foreignCommunity}`));
  await expect(page.getByRole("heading", { name: "Beta club" })).toBeVisible();

  await rail(page)
    .getByRole("link", { name: /Direct messages/ })
    .click();
  const dms = page.getByRole("navigation", { name: "Direct messages" });
  const foreignRow = dms.getByRole("link", { name: /Hostess/ });
  await expect(foreignRow).toContainText(foreignDomain);
  await expect(dms.getByRole("link").first()).toContainText("Hostess");
  await foreignRow.click();
  await expect(page).toHaveURL(/\/at\/beta\.example\/dms\//);
});

test("someone adds another server from the rail", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: false }));
  await expect(rail(page).getByRole("row", { name: /^Family/ })).toBeVisible();
  await expect(rail(page).getByRole("row", { name: /Beta club/ })).toHaveCount(0);
  await rail(page).getByRole("button", { name: "Create or join a community" }).click();
  await page.getByRole("button", { name: /Use another server/ }).click();
  await page.getByRole("textbox", { name: "Server" }).fill(foreignDomain);
  await page.getByRole("dialog").getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/at\/beta\.example\/communities\//);
});

test("a new message starts on the deployment chosen", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  await expect(
    rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` }),
  ).toBeVisible();
  await rail(page)
    .getByRole("link", { name: /Direct messages/ })
    .click();
  await page.getByRole("button", { name: "New message" }).click();
  const picker = page.getByRole("dialog", { name: "New message" });
  await picker.getByRole("radio", { name: foreignDomain }).check({ force: true });
  await picker.getByRole("option", { name: /Hostess/ }).click();
  await picker.getByRole("button", { name: "Message", exact: true }).click();
  await expect(page).toHaveURL(/\/at\/beta\.example\/dms\//);
});
