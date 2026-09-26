import { expect, test } from "@playwright/test";
import { stubSignedInBackend, uuid } from "./stubs";

/**
 * Exercises the login screen against a stubbed server so the suite runs without infrastructure.
 * Real end-to-end coverage against a running Aspen server belongs in a separate, opt-in project.
 */
test.describe("login", () => {
  test("shows the server's Problem text when credentials are rejected", async ({ page }) => {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 401,
        contentType: "application/problem+json",
        body: JSON.stringify({
          code: "invalidCredentials",
          title: "Incorrect username or password.",
          status: 401,
        }),
      }),
    );
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("wrong");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByRole("alert")).toHaveText("Incorrect username or password.");
  });

  test("signs in and loads the caller's profile", async ({ page }) => {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          userId: uuid,
          refreshToken: "r",
          sessionToken: "s",
          sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
        }),
      }),
    );
    const authorizationSeen: (string | undefined)[] = [];
    page.on("request", (request) => {
      if (request.url().endsWith("/api/v1/users/%40me")) {
        authorizationSeen.push(request.headers().authorization);
      }
    });
    await stubSignedInBackend(page, "kate");
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("hunter22");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    // React's development-mode double effect may bootstrap twice; every read must carry the token.
    expect(authorizationSeen.length).toBeGreaterThan(0);
    expect(authorizationSeen.every((h) => h === "Bearer s")).toBe(true);
  });
});
