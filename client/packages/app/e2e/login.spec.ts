import { expect, test, type Page } from "@playwright/test";
import {
  signedIn,
  stubAuthMethods,
  stubDeploymentProfile,
  stubSignedInBackend,
  uuid,
} from "./stubs";

/** Aspen's icon, which is small enough that Vite inlines it as an SVG data URL. */
const ASPEN_ICON = /^data:image\/svg\+xml,.*aria-label='Aspen'/;

/** The icon's side: 256 pixels from Tailwind's `md` (48rem) up, half that on a phone. */
function iconPx(page: Page): number {
  return (page.viewportSize()?.width ?? 0) >= 768 ? 256 : 128;
}

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
    // Above the welcome, at 256 pixels, or half that on a phone.
    const [iconBox, welcomeBox] = await Promise.all([
      icon.boundingBox(),
      page.getByText(/^Welcome to/).boundingBox(),
    ]);
    expect(iconBox?.width).toBe(iconPx(page));
    expect((iconBox?.y ?? 0) + (iconBox?.height ?? 0)).toBeLessThanOrEqual(welcomeBox?.y ?? 0);
    // The address still shows, but the web client's server is the one serving it.
    await expect(page.getByText("Server:")).toBeVisible();
    await expect(page.getByRole("button", { name: "Change" })).toHaveCount(0);
  });

  test("welcomes people generally, under Aspen's icon, to a deployment with neither", async ({
    page,
  }) => {
    await page.goto("/");
    await expect(
      page.getByText("Welcome to our Aspen Chat instance.", { exact: true }),
    ).toBeVisible();
    const icon = page.locator("main img");
    await expect(icon).toHaveCount(1);
    await expect(icon).toHaveAttribute("src", ASPEN_ICON);
    expect((await icon.boundingBox())?.width).toBe(iconPx(page));
    // The create-account screen welcomes them the same way.
    await page.getByRole("button", { name: "Create one" }).click();
    await expect(page.getByRole("heading", { name: "Create an account" })).toBeVisible();
    await expect(
      page.getByText("Welcome to our Aspen Chat instance.", { exact: true }),
    ).toBeVisible();
  });

  test("shows Aspen's icon in place of a deployment icon that fails to load", async ({ page }) => {
    const iconUrl = "https://media.example.org/icons/missing.png";
    await page.route(iconUrl, (route) => route.fulfill({ status: 404 }));
    await stubDeploymentProfile(page, {
      displayName: "Kiesel Family Chat",
      icon: { id: uuid, mimeType: "image/png", downloadUrl: iconUrl },
    });
    await page.goto("/");
    await expect(page.locator("main img")).toHaveAttribute("src", ASPEN_ICON);
  });

  test("the page's ground reaches the foot of a sign-in screen taller than the window", async ({
    page,
  }) => {
    // Dark, where no palette's ground is the browser's own.
    await page.emulateMedia({ colorScheme: "dark" });
    const height = 320;
    await page.setViewportSize({ width: page.viewportSize()?.width ?? 0, height });
    await page.goto("/");
    await expect(page.getByRole("link", { name: "Privacy Policy" })).toBeVisible();
    // A pixel at the screen's edge, in its padding, where only the ground is drawn.
    const ground = (y: number) =>
      page.screenshot({ clip: { x: 2, y, width: 1, height: 1 }, animations: "disabled" });
    const head = await ground(2);
    await page.evaluate(() => {
      window.scrollTo(0, document.documentElement.scrollHeight);
    });
    expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
    expect((await ground(height - 2)).equals(head)).toBe(true);
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

  /** Refuses the sign-in for lacking a device credential, so the app sets out to earn one. */
  async function needingDeviceCredential(page: Page) {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 401,
        contentType: "application/problem+json",
        body: JSON.stringify({
          code: "deviceCredentialRequired",
          title: "This app needs to register this device with the server first.",
          detail: "It does this on its own; if you see this, update the app.",
          status: 401,
        }),
      }),
    );
  }

  test("says the server could not be reached when earning a device credential gets no answer", async ({
    page,
  }) => {
    await needingDeviceCredential(page);
    await page.route("**/api/v1/auth/device-challenges", (route) => route.abort("failed"));
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("hunter22");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("alert")).toHaveText(
      `${new URL(page.url()).host} couldn't be reached. Check your connection, then try again.`,
    );
  });

  test("shows why earning a device credential was refused, not that one was needed", async ({
    page,
  }) => {
    await needingDeviceCredential(page);
    await page.route("**/api/v1/auth/device-challenges", (route) =>
      route.fulfill({
        status: 429,
        contentType: "application/problem+json",
        headers: { "Retry-After": "30" },
        body: JSON.stringify({
          code: "rateLimited",
          title: "Too many requests.",
          detail: "Wait 30 seconds, then try again.",
          status: 429,
        }),
      }),
    );
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("hunter22");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("alert")).toHaveText("Wait 30 seconds, then try again.");
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

  test("signs out from the home of someone in no community", async ({ page }) => {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: signedIn(),
      }),
    );
    const logouts: unknown[] = [];
    await page.route("**/api/v1/auth/logout", (route) => {
      logouts.push(route.request().postDataJSON());
      return route.fulfill({ status: 204 });
    });
    await stubSignedInBackend(page, "kate");
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("hunter22");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
    expect(logouts).toEqual([{ refreshToken: "r" }]);
  });

  test("keeps a dialog open on the home while a failed first read is retried", async ({ page }) => {
    await page.route("**/api/v1/auth/login", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: signedIn(),
      }),
    );
    // Only the profile is answered, so every bootstrap fails and is tried again by itself.
    await stubSignedInBackend(page, "kate");
    let reads = 0;
    page.on("request", (request) => {
      if (/\/api\/v1\/users\/(%40|@)me\/communities$/.test(new URL(request.url()).pathname)) {
        reads += 1;
      }
    });
    await page.goto("/");
    await page.getByLabel("Username").fill("kate");
    await page.getByLabel("Password").fill("hunter22");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
    await page.getByRole("button", { name: "Create a community" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    const opened = reads;
    await expect.poll(() => reads, { timeout: 15_000 }).toBeGreaterThan(opened + 1);
    await expect(dialog).toBeVisible();
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
