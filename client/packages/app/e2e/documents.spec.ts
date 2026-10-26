import { expect, test, type Page } from "@playwright/test";
import { stubAuthMethods } from "./stubs";
import { signInToWorld } from "./world";

/**
 * The deployment's Privacy Policy and Terms of Use: the links at the foot of the sign-in screen
 * and in About Aspen, and their pages. The deployment here has published a Privacy Policy and no
 * Terms of Use.
 */

const POLICY = [
  "# Privacy Policy",
  "",
  "We keep what you post. [Write to us](mailto:privacy@example.org) with questions.",
  "",
  "![a tracking pixel](https://tracker.example.org/pixel.png)",
].join("\n");

async function stubDocuments(page: Page) {
  await page.route(/\/api\/v1\/deployment\/documents\/(privacyPolicy|termsOfUse)$/, (route) => {
    const kind = new URL(route.request().url()).pathname.split("/").pop();
    return route.fulfill({
      json:
        kind === "privacyPolicy"
          ? { kind, body: POLICY, updatedAt: "2026-10-20T12:00:00Z" }
          : { kind, body: null, updatedAt: null },
    });
  });
}

test("signed out, the sign-in screen's foot leads to the deployment's documents and back", async ({
  page,
}) => {
  await stubAuthMethods(page);
  await stubDocuments(page);
  await page.goto("/");
  const links = page.getByRole("navigation", { name: "The deployment's policies" });
  await links.getByRole("link", { name: "Privacy Policy" }).click();
  await expect(page).toHaveURL(/\/privacy$/);
  await expect(page.getByRole("heading", { name: "Privacy Policy", level: 1 })).toBeVisible();
  // The document's own top heading sits under the page's.
  await expect(page.getByRole("heading", { name: "Privacy Policy", level: 2 })).toBeVisible();
  await expect(page.getByText("Last updated October 20, 2026")).toBeVisible();
  await expect(page.getByRole("link", { name: "Write to us" })).toHaveAttribute(
    "href",
    "mailto:privacy@example.org",
  );
  // A picture is never loaded; its description stands in its place.
  await expect(page.getByText("a tracking pixel")).toBeVisible();
  await expect(page.locator("article img")).toHaveCount(0);

  await page.getByRole("link", { name: "Back to sign in" }).click();
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  await links.getByRole("link", { name: "Terms of Use" }).click();
  await expect(page.getByRole("heading", { name: "Terms of Use", level: 1 })).toBeVisible();
  await expect(page.getByText("This deployment has not published Terms of Use.")).toBeVisible();
});

test("signed in, About Aspen leads to them and closes Settings", async ({ page }) => {
  await signInToWorld(page, stubDocuments);
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Settings", exact: true })
    .getByRole("button", { name: "About Aspen" })
    .click();
  const policies = page
    .getByRole("dialog", { name: "About Aspen" })
    .getByRole("region", { name: "Family Server's policies" });
  await policies.getByRole("link", { name: "Terms of Use" }).click();
  await expect(page.getByRole("dialog", { name: "Settings", exact: true })).toHaveCount(0);
  await expect(page).toHaveURL(/\/terms$/);
  await expect(page.getByText("This deployment has not published Terms of Use.")).toBeVisible();
});
