import { expect, test } from "@playwright/test";
import {
  signedIn,
  stubAuthMethods,
  stubDeploymentProfile,
  stubSignedInBackend,
  uuid,
} from "./stubs";

/** A one-pixel PNG, for the deployment's icon. */
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
);

/**
 * Exercises the login screen against a stubbed server so the suite runs without infrastructure.
 * Real end-to-end coverage against a running Aspen server belongs in a separate, opt-in project.
 */
test.describe("login", () => {
  test.beforeEach(async ({ page }) => {
    await stubAuthMethods(page);
  });

  test("welcomes people to the deployment by name, under its icon", async ({ page }) => {
    const iconUrl = "https://media.example.org/icons/deployment.png";
    await page.route(iconUrl, (route) =>
      route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
    );
    await stubDeploymentProfile(page, {
      displayName: "Kiesel Family Chat",
      icon: { id: uuid, mimeType: "image/png", downloadUrl: iconUrl },
    });
    await page.goto("/");
    await expect(
      page.getByText("Welcome to Kiesel Family Chat, an Aspen Chat instance.", { exact: true }),
    ).toBeVisible();
    const icon = page.locator(`img[src="${iconUrl}"]`);
    await expect(icon).toBeVisible();
    // Above the welcome, at 256 pixels where the screen has room for them.
    const [iconBox, welcomeBox] = await Promise.all([
      icon.boundingBox(),
      page.getByText(/^Welcome to/).boundingBox(),
    ]);
    expect(iconBox?.width).toBe(256);
    expect((iconBox?.y ?? 0) + (iconBox?.height ?? 0)).toBeLessThanOrEqual(welcomeBox?.y ?? 0);
    // The address still shows, but the web client's server is the one serving it.
    await expect(page.getByText("Server:")).toBeVisible();
    await expect(page.getByRole("button", { name: "Change" })).toHaveCount(0);
  });

  test("welcomes people generally to a deployment with no name or icon", async ({ page }) => {
    await page.goto("/");
    await expect(
      page.getByText("Welcome to our Aspen Chat instance.", { exact: true }),
    ).toBeVisible();
    await expect(page.locator("main img")).toHaveCount(0);
    // The create-account screen welcomes them the same way.
    await page.getByRole("button", { name: "Create one" }).click();
    await expect(page.getByRole("heading", { name: "Create an account" })).toBeVisible();
    await expect(
      page.getByText("Welcome to our Aspen Chat instance.", { exact: true }),
    ).toBeVisible();
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
