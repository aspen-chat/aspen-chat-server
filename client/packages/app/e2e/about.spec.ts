import { expect, test, type Page } from "@playwright/test";
import { stubAuthMethods } from "./stubs";
import { signInToWorld } from "./world";

/**
 * About Aspen, opened from Settings, and the Open Source Attributions page, against the stubbed world in
 * `world.ts`. The page's list is the one the dev server makes from this checkout
 * (`attributions/plugin.ts`), so these look for packages the app is sure to have.
 */

/**
 * Waits for the attributions page's list, and expects each part of Aspen in it. A development
 * server without the Rust toolchain (Playwright's Docker image) cannot list the servers' crates,
 * and says so, so the servers' part is looked for only where the list is complete.
 */
async function expectEveryPart(page: Page) {
  await expect(page.getByRole("heading", { name: /^The app/, level: 2 })).toBeVisible({
    timeout: 60_000,
  });
  const incomplete = page
    .getByRole("status")
    .filter({ hasText: "This development build couldn't list every package." });
  const parts = ["The desktop app", "The phone apps"];
  if (!(await incomplete.isVisible())) {
    parts.push("The servers");
  }
  for (const part of parts) {
    await expect(
      page.getByRole("heading", { name: new RegExp(`^${part}`), level: 2 }),
    ).toBeVisible();
  }
}

async function openAbout(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Settings", exact: true })
    .getByRole("button", { name: "About Aspen" })
    .click();
  return page.getByRole("dialog", { name: "About Aspen" });
}

test("About Aspen names the deployment and the versions of the app, server, and protocol", async ({
  page,
}) => {
  await signInToWorld(page);
  const about = await openAbout(page);
  await expect(about.getByText("You're on Family Server.")).toBeVisible();
  await expect(about.locator("img")).toHaveAttribute("src", /^data:image\/svg\+xml/);
  const versions = about.locator("dl");
  await expect(versions).toContainText(/Aspen app\s*0\.1\.0/);
  await expect(versions).toContainText(/Server\s*aspen 0\.1\.0/);
  await expect(versions).toContainText(/Protocol\s*This app 1, server 1/);
  const rights = about.getByRole("region", { name: "Your rights" });
  await expect(rights).toContainText("Mozilla Public License 2.0");
  await expect(rights.getByRole("link", { name: "Read the license" })).toHaveAttribute(
    "href",
    "https://www.mozilla.org/MPL/2.0/",
  );
  await expect(rights.getByRole("link", { name: "Source code" })).toHaveAttribute(
    "href",
    /^https:\/\/github\.com\/aspen-chat\/aspen-chat-server/,
  );
});

test("the attributions link closes About Aspen and Settings and lists every part's packages with their licenses", async ({
  page,
}) => {
  test.slow(); // The dev server reads every package's license files the first time.
  await signInToWorld(page);
  const about = await openAbout(page);
  await about.getByRole("link", { name: "Open source attributions" }).click();
  await expect(page.getByRole("dialog", { name: "Settings", exact: true })).toHaveCount(0);
  await expect(page.getByRole("dialog", { name: "About Aspen" })).toHaveCount(0);
  await expect(page).toHaveURL(/\/attributions$/);
  await expect(
    page.getByRole("heading", { name: "Open source attributions", level: 1 }),
  ).toBeVisible();
  await expectEveryPart(page);

  await page.getByRole("searchbox", { name: "Find a package" }).fill("react-aria-components");
  const app = page.getByRole("region", { name: /^The app/ });
  const row = app.getByRole("listitem").filter({ hasText: /^react-aria-components/ });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("License: Apache-2.0");
  await expect(page.getByRole("region", { name: /^The servers/ })).toHaveCount(0);
  await row.getByRole("button", { name: "License text" }).click();
  await expect(row.getByText("Apache License", { exact: false }).first()).toBeVisible();

  await page.getByRole("searchbox", { name: "Find a package" }).fill("Chromium");
  await expect(
    page.getByText("Chromium's notices are too long to show here.", { exact: false }),
  ).toBeVisible();

  await page.getByRole("searchbox", { name: "Find a package" }).fill("no package is named this");
  await expect(page.getByText("No package matches that.")).toBeVisible();
});

test("the attributions page opens signed out, and leads back to signing in", async ({ page }) => {
  test.slow();
  await stubAuthMethods(page);
  await page.goto("/attributions");
  await expect(
    page.getByRole("heading", { name: "Open source attributions", level: 1 }),
  ).toBeVisible();
  await expectEveryPart(page);
  await page.getByRole("link", { name: "Back to sign in" }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
});
