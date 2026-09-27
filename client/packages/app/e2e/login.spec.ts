import { expect, test } from "@playwright/test";
import { signedIn, stubAuthMethods, stubSignedInBackend } from "./stubs";

/**
 * Exercises the login screen against a stubbed server so the suite runs without infrastructure.
 * Real end-to-end coverage against a running Aspen server belongs in a separate, opt-in project.
 */
test.describe("login", () => {
  test.beforeEach(async ({ page }) => {
    await stubAuthMethods(page);
  });

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
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("alert")).toHaveText("Incorrect username or password.");
  });

  test("signs in and loads the caller's profile", async ({ page }) => {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: signedIn(),
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
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    // React's development-mode double effect may bootstrap twice; every read must carry the token.
    expect(authorizationSeen.length).toBeGreaterThan(0);
    expect(authorizationSeen.every((h) => h === "Bearer s")).toBe(true);
  });

  test("finishes a two-factor sign-in with an authenticator code", async ({ page }) => {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          status: "secondFactorRequired",
          ticket: "ticket-1",
          methods: ["totp", "recoveryCode"],
        }),
      }),
    );
    const attempts: unknown[] = [];
    await page.route("**/api/v1/auth/login/second-factor", (route) => {
      attempts.push(route.request().postDataJSON());
      return attempts.length === 1
        ? route.fulfill({
            status: 403,
            contentType: "application/problem+json",
            body: JSON.stringify({
              code: "verificationFailed",
              title: "That password or code isn't right, or has already been used.",
              status: 403,
            }),
          })
        : route.fulfill({ status: 200, contentType: "application/json", body: signedIn(null) });
    });
    await stubSignedInBackend(page, "kate");
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("hunter22");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Two-factor sign-in" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Use a passkey" })).toHaveCount(0);
    await page.getByLabel("Authenticator code").fill("000000");
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page.getByRole("alert")).toHaveText(
      "That password or code isn't right, or has already been used.",
    );
    await page.getByRole("button", { name: "Use a recovery code" }).click();
    await page.getByLabel("Recovery code").fill("abcde-fghjk");
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    expect(attempts).toEqual([
      { ticket: "ticket-1", method: "totp", code: "000000" },
      { ticket: "ticket-1", method: "recoveryCode", code: "abcde-fghjk" },
    ]);
  });
});
