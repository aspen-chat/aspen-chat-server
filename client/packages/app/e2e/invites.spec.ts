import { expect, test } from "@playwright/test";
import { community, inviteCode, signInToWorld } from "./world";

/**
 * A community's invites, against the stubbed world, where the caller manages its invites: one
 * made with a code of their choosing, and one's expiry changed.
 */

test("a manager of invites chooses a new invite's code, and changes when one expires", async ({
  page,
}) => {
  await signInToWorld(page);
  const made: unknown[] = [];
  const changed: unknown[] = [];
  await page.route("**/api/v1/communities/*/invites", (route) => {
    if (route.request().method() !== "POST") {
      return route.fallback();
    }
    made.push(route.request().postDataJSON());
    return route.fulfill({
      status: 201,
      json: {
        code: "family",
        community,
        createdBy: "0190f0a0-0000-7000-8000-000000000001",
        createdAt: new Date().toISOString(),
        expiresAt: null,
      },
    });
  });
  await page.route(`**/api/v1/invites/${inviteCode}`, (route) => {
    if (route.request().method() !== "PATCH") {
      return route.fallback();
    }
    const body = route.request().postDataJSON() as { expiresAt: string | null };
    changed.push(body);
    return route.fulfill({
      json: {
        code: inviteCode,
        community,
        createdBy: "0190f0a0-0000-7000-8000-000000000001",
        createdAt: new Date().toISOString(),
        expiresAt: body.expiresAt,
      },
    });
  });
  await page.getByRole("button", { name: "Invite people" }).first().click();
  const dialog = page.getByRole("dialog", { name: /^Invite people to / });
  await dialog.getByRole("textbox", { name: "Code (optional)" }).fill("family");
  await dialog.getByRole("button", { name: "Create invite" }).click();
  await expect.poll(() => made).toEqual([{ expiresAt: expect.any(String), customCode: "family" }]);

  await dialog.getByRole("button", { name: `Change when invite ${inviteCode} expires` }).click();
  await page.getByRole("menuitem", { name: "Never" }).click();
  await expect.poll(() => changed).toEqual([{ expiresAt: null }]);
  await expect(
    dialog.getByRole("listitem").filter({ hasText: inviteCode }).getByText("Never expires"),
  ).toBeVisible();
});
