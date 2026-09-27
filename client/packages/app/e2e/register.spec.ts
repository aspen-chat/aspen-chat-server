import { expect, test, type Page } from "@playwright/test";
import { signedIn, stubAuthMethods, stubSignedInBackend, uuid } from "./stubs";

async function openRegistration(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Create one" }).click();
  await expect(page.getByRole("heading", { name: "Create an account" })).toBeVisible();
}

test.describe("registration", () => {
  test.beforeEach(async ({ page }) => {
    await stubAuthMethods(page);
  });

  test("creates the account, signs in, and lands on the welcome screen", async ({ page }) => {
    const bodies: unknown[] = [];
    await page.route("**/api/v1/users", (route) => {
      bodies.push(route.request().postDataJSON());
      return route.fulfill({
        status: 201,
        contentType: "application/json",
        headers: { location: `/api/v1/users/${uuid}` },
        body: JSON.stringify({ id: uuid, name: "kate", icon: null, onlineStatus: "offline" }),
      });
    });
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: signedIn(),
      }),
    );
    await stubSignedInBackend(page, "kate");
    await openRegistration(page);
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password", { exact: true }).fill("hunter22!");
    await page.getByLabel("Confirm password").fill("hunter22!");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    expect(bodies).toEqual([{ name: "kate", password: "hunter22!" }]);
  });

  test("refuses mismatched passwords without calling the server", async ({ page }) => {
    let called = false;
    await page.route("**/api/v1/users", (route) => {
      called = true;
      return route.abort();
    });
    await openRegistration(page);
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password", { exact: true }).fill("hunter22!");
    await page.getByLabel("Confirm password").fill("different1");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByText("The passwords do not match.")).toBeVisible();
    expect(called).toBe(false);
  });

  test("attaches a taken username to the username field", async ({ page }) => {
    await page.route("**/api/v1/users", (route) =>
      route.fulfill({
        status: 409,
        contentType: "application/problem+json",
        body: JSON.stringify({
          code: "usernameTaken",
          title: "Username already in use, pick a different username.",
          status: 409,
        }),
      }),
    );
    await openRegistration(page);
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password", { exact: true }).fill("hunter22!");
    await page.getByLabel("Confirm password").fill("hunter22!");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(
      page.getByText("Username already in use, pick a different username."),
    ).toBeVisible();
    await expect(page.getByRole("heading", { name: "Create an account" })).toBeVisible();
  });
});
