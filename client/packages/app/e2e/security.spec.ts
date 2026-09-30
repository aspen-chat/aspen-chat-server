import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Sign-in and security, against the stubbed world in `world.ts`, with the security settings and
 * the password endpoint answered here: the password is "hunter22", and two-factor sign-in is off.
 */

const settings = {
  passkeys: [],
  recoveryCodesRemaining: 0,
  totp: false,
  twoFactorEnabled: false,
  twoFactorRequired: false,
  verifiedUntil: new Date(Date.now() + 600_000).toISOString(),
};

async function stubSecurity(page: Page, changes: unknown[]) {
  await page.route(/\/api\/v1\/users\/(%40|@)me\/security$/, (route) =>
    route.fulfill({ json: settings }),
  );
  await page.route(/\/api\/v1\/users\/(%40|@)me\/password$/, (route) => {
    const body = route.request().postDataJSON() as { oldPassword: string; newPassword: string };
    changes.push(body);
    if (body.oldPassword !== "hunter22") {
      return route.fulfill({
        status: 403,
        contentType: "application/problem+json",
        body: JSON.stringify({
          code: "oldPasswordIncorrect",
          title: "Incorrect password",
          detail: "The current password is incorrect.",
          status: 403,
        }),
      });
    }
    return route.fulfill({ status: 204 });
  });
}

async function openSecurity(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Sign-in and security" }).click();
  return page
    .getByRole("dialog", { name: "Sign-in and security" })
    .getByRole("region", { name: "Password" });
}

test("a user changes their own password, and a wrong current one is refused at its field", async ({
  page,
}) => {
  const changes: unknown[] = [];
  await signInToWorld(page, (p) => stubSecurity(p, changes));
  const section = await openSecurity(page);
  await section.getByRole("button", { name: "Change password" }).click();
  const fill = async (current: string, next: string, again: string) => {
    await section.getByLabel("Current password").fill(current);
    await section.getByLabel("New password", { exact: true }).fill(next);
    await section.getByLabel("Confirm new password").fill(again);
    await section.getByRole("button", { name: "Change password" }).click();
  };

  await fill("hunter22", "short", "short");
  await expect(section.getByText("At least 8 characters.")).toBeVisible();
  await fill("hunter22", "a new password", "another one");
  await expect(section.getByText("The passwords do not match.")).toBeVisible();
  expect(changes).toHaveLength(0);

  await fill("wrong one", "a new password", "a new password");
  await expect(section.getByText("The current password is incorrect.")).toBeVisible();
  await expect(section.getByLabel("Current password")).toHaveAttribute("aria-invalid", "true");

  await fill("hunter22", "a new password", "a new password");
  await expect(
    section.getByText("Password changed. You've been signed out everywhere else."),
  ).toBeVisible();
  expect(changes.at(-1)).toEqual({ oldPassword: "hunter22", newPassword: "a new password" });
});
