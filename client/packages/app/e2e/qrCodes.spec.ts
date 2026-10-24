import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { stubAuthMethods } from "./stubs";
import { settleAnimations, signInToWorld } from "./world";

/** What axe finds wrong with the open dialog, one line per element. */
async function problems(page: Page): Promise<string[]> {
  await settleAnimations(page);
  await page.mouse.move(0, 0);
  const audit = await new AxeBuilder({ page }).include('[role="dialog"]').analyze();
  return audit.violations.flatMap((v) => v.nodes.map((n) => `${v.id} at ${n.target.join(" ")}`));
}

const LINK_ID = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-abcde";

test("a computer that is not signed in shows a sign-in code for a phone, and gives it up on closing", async ({
  page,
}) => {
  await stubAuthMethods(page);
  const claims: string[] = [];
  let cancelled = false;
  let asked: unknown = null;
  await page.route("**/api/v1/auth/device-links", (route) => {
    asked = route.request().postDataJSON();
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        id: LINK_ID,
        kind: "request",
        expiresAt: new Date(Date.now() + 300_000).toISOString(),
        code: "BCDF-GHJK",
      }),
    });
  });
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}/claim`, (route) => {
    claims.push((route.request().postDataJSON() as { codeVerifier: string }).codeVerifier);
    // Scanned once the second ask comes.
    const body =
      claims.length < 2
        ? { status: "waiting" }
        : { status: "scanned", deviceName: "Aspen on Android" };
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
  });
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}`, (route) => {
    cancelled = route.request().method() === "DELETE";
    return route.fulfill({ status: 204 });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Sign in with your phone" }).click();
  const dialog = page.getByRole("dialog", { name: "Sign in with your phone" });
  await expect(dialog.getByRole("img", { name: "Sign-in code" })).toBeVisible();
  await expect(dialog).toContainText("This code is a secret");
  await expect(dialog.getByText(/^Expires in \d+ s$/)).toBeVisible();
  // Beside the QR code, the user code to type where it cannot be scanned.
  await expect(dialog.getByText("BCDF-GHJK", { exact: true })).toBeVisible();
  expect(asked).toMatchObject({ withCode: true });
  expect(await problems(page)).toEqual([]);
  await expect(dialog).toContainText("Aspen on Android scanned the code. Confirm on it");
  // Every claim proves the same verifier, which only this page holds.
  expect(new Set(claims).size).toBe(1);
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect.poll(() => cancelled).toBe(true);
});

test("a code nobody scans in time blurs, and a new one replaces it", async ({ page }) => {
  await stubAuthMethods(page);
  // Codes expire at once until a new one is asked for.
  let lasting = false;
  await page.route("**/api/v1/auth/device-links", (route) =>
    route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        id: LINK_ID,
        kind: "request",
        expiresAt: new Date(Date.now() + (lasting ? 60_000 : 500)).toISOString(),
      }),
    }),
  );
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}/claim`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: '{"status":"waiting"}' }),
  );
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}`, (route) =>
    route.fulfill({ status: 204 }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Sign in with your phone" }).click();
  const dialog = page.getByRole("dialog", { name: "Sign in with your phone" });
  await expect(dialog.getByText("This code expired.")).toBeVisible();
  lasting = true;
  await dialog.getByRole("button", { name: "New code" }).click();
  await expect(dialog.getByText(/^Expires in \d+ s$/)).toBeVisible();
  await expect(dialog.getByText("This code expired.")).toHaveCount(0);
});

test("an invite shows its QR code when made, and on request, with a choice of downloads", async ({
  page,
}) => {
  await signInToWorld(page);
  // The world lists its community's invites; making one is answered here.
  await page.route("**/api/v1/communities/*/invites", (route) => {
    if (route.request().method() !== "POST") {
      return route.fallback();
    }
    const community = /communities\/([^/]+)\/invites/.exec(route.request().url())?.[1] ?? "";
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        code: "MadeHere1",
        community,
        createdBy: "0190f0a0-0000-7000-8000-000000000001",
        createdAt: new Date().toISOString(),
        expiresAt: null,
      }),
    });
  });
  await page.getByRole("button", { name: "Invite people" }).first().click();
  const dialog = page.getByRole("dialog", { name: /^Invite people to / });
  await dialog.getByRole("button", { name: "Create invite" }).click();
  const code = dialog.getByRole("img", { name: /^QR code for invite / });
  await expect(code).toHaveCount(1);
  expect(await problems(page)).toEqual([]);
  // The code's toggle hides it, and another invite's shows that one's instead.
  const shown = dialog.getByRole("button", { name: "Show QR code", expanded: true });
  await shown.click();
  await expect(code).toHaveCount(0);
  await dialog.getByRole("button", { name: "Show QR code" }).first().click();
  await expect(code).toHaveCount(1);
  await dialog.getByRole("button", { name: "Download" }).click();
  await expect(page.getByRole("menuitem", { name: "PNG image" })).toBeVisible();
  const [svg] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("menuitem", { name: "SVG image" }).click(),
  ]);
  expect(svg.suggestedFilename()).toMatch(/^aspen-invite-.+\.svg$/);
});

test("a computer whose code was typed shows the account offered, and claims it only once confirmed", async ({
  page,
}) => {
  await stubAuthMethods(page);
  const accounts: (string | undefined)[] = [];
  await page.route("**/api/v1/auth/device-links", (route) =>
    route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        id: LINK_ID,
        kind: "request",
        expiresAt: new Date(Date.now() + 300_000).toISOString(),
        code: "BCDF-GHJK",
      }),
    }),
  );
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}/claim`, (route) => {
    accounts.push((route.request().postDataJSON() as { account?: string }).account);
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "confirmAccount",
        userId: "0190f0a0-0000-7000-8000-0000000000aa",
        userName: "ada",
        displayName: "Ada Lovelace",
        icon: null,
      }),
    });
  });
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}`, (route) =>
    route.fulfill({ status: 204 }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Sign in with your phone" }).click();
  const dialog = page.getByRole("dialog", { name: "Sign in with your phone" });
  await expect(dialog.getByText("Ada Lovelace")).toBeVisible();
  await expect(dialog.getByText("@ada")).toBeVisible();
  await expect(dialog).toContainText("Only continue if this is your own account.");
  expect(await problems(page)).toEqual([]);
  // Until it is confirmed, no claim names the account.
  expect(accounts.every((account) => account === undefined)).toBe(true);
  await dialog.getByRole("button", { name: "This is my account" }).click();
  await expect(dialog.getByText("Signing in as Ada Lovelace…")).toBeVisible();
  await expect.poll(() => accounts.at(-1)).toBe("0190f0a0-0000-7000-8000-0000000000aa");
});

test("a device that is not signed in types a code instead, and waits for the other to confirm it", async ({
  page,
}) => {
  await stubAuthMethods(page);
  const entries: unknown[] = [];
  await page.route("**/api/v1/auth/device-link-entries", (route) => {
    entries.push(route.request().postDataJSON());
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ id: LINK_ID, kind: "offer", userName: "ada", displayName: "Ada" }),
    });
  });
  await page.route(`**/api/v1/auth/device-links/${LINK_ID}/claim`, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "scanned", deviceName: "Television" }),
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Type a code instead" }).click();
  const dialog = page.getByRole("dialog", { name: "Type a sign-in code" });
  expect(await problems(page)).toEqual([]);
  const field = dialog.getByLabel("Code");
  await field.fill("bcdf-ghj");
  await dialog.getByRole("button", { name: "Continue" }).click();
  await expect(dialog.getByText("A sign-in code is 8 letters")).toBeVisible();
  // A code that cannot be one is not sent, since a wrong one counts against this device.
  expect(entries).toHaveLength(0);
  await field.fill("bcdf ghjk");
  await dialog.getByRole("button", { name: "Continue" }).click();
  await expect(dialog.getByText("Signing in as Ada.")).toBeVisible();
  expect(entries).toEqual([
    expect.objectContaining({ code: "BCDFGHJK", deviceName: expect.any(String) }),
  ]);
  expect((entries[0] as { codeChallenge?: string }).codeChallenge).toMatch(/^[A-Za-z0-9_-]{43}$/);
});

test("a wrong code is said at the field, and editing it clears that", async ({ page }) => {
  await stubAuthMethods(page);
  await page.route("**/api/v1/auth/device-link-entries", (route) =>
    route.fulfill({
      status: 404,
      contentType: "application/problem+json",
      body: JSON.stringify({
        code: "deviceLinkCodeUnknown",
        title: "That sign-in code isn't right.",
        detail: "No sign-in code waiting for this device matches what you typed.",
        status: 404,
      }),
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Type a code instead" }).click();
  const dialog = page.getByRole("dialog", { name: "Type a sign-in code" });
  const field = dialog.getByLabel("Code");
  await field.fill("BCDF-GHJK");
  await dialog.getByRole("button", { name: "Continue" }).click();
  await expect(dialog.getByText("No sign-in code waiting for this device matches")).toBeVisible();
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await field.fill("BCDF-GHJL");
  await expect(dialog.getByText("No sign-in code waiting for this device matches")).toHaveCount(0);
});

test("a signed-in device types a code asking for a sign-in, and is told the other confirms the account", async ({
  page,
}) => {
  const approvals: string[] = [];
  await signInToWorld(page, async (p) => {
    await p.route(/\/api\/v1\/users\/(%40|@)me\/security$/, (route) =>
      route.fulfill({
        json: {
          passkeys: [],
          recoveryCodesRemaining: 0,
          totp: false,
          twoFactorEnabled: false,
          twoFactorRequired: false,
          verifiedUntil: new Date(Date.now() + 600_000).toISOString(),
        },
      }),
    );
    await p.route("**/api/v1/auth/device-link-entries", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ id: LINK_ID, kind: "request", deviceName: "Television" }),
      }),
    );
    await p.route(`**/api/v1/auth/device-links/${LINK_ID}/approval`, (route) => {
      approvals.push(route.request().method());
      return route.fulfill({ status: 204 });
    });
  });
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Sign-in and security" }).click();
  await page
    .getByRole("dialog", { name: "Sign-in and security" })
    .getByRole("button", { name: "Type a code instead" })
    .click();
  const dialog = page.getByRole("dialog", { name: "Type a sign-in code" });
  await dialog.getByLabel("Code").fill("BCDF-GHJK");
  await dialog.getByRole("button", { name: "Continue" }).click();
  await expect(dialog.getByText("Sign in Television to your account?")).toBeVisible();
  expect(await problems(page)).toEqual([]);
  await dialog.getByRole("button", { name: "Sign it in" }).click();
  await expect(
    dialog.getByText("Confirm on Television that it's your account to finish signing it in."),
  ).toBeVisible();
  expect(approvals).toEqual(["PUT"]);
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(dialog).toHaveCount(0);
});
