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
  await page.route("**/api/v1/auth/device-links", (route) =>
    route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        id: LINK_ID,
        kind: "request",
        expiresAt: new Date(Date.now() + 60_000).toISOString(),
      }),
    }),
  );
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
