import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Banning from community settings: the organiser bans the helper with a reason and for a
 * day, who leaves the member list and appears among the banned, and lifts the ban again. The
 * organiser lacks Manage messages, so the ban offers no deletion of messages.
 */
test("a member is banned with a reason, listed as banned, and the ban is lifted", async ({
  page,
}) => {
  await signInToWorld(page);
  await page.getByRole("button", { name: "Community settings" }).click();
  const dialog = page.getByRole("dialog", { name: /settings/ });
  await dialog.getByRole("tab", { name: "Members" }).click();
  const members = dialog.getByRole("list", { name: "Members" });
  const helper = members.getByRole("listitem").filter({ hasText: "Helper" });
  await expect(helper).toBeVisible();
  await expect(dialog.getByText("No one is banned.")).toBeVisible();
  // Bob owns the community, so no ban is offered on him.
  await expect(
    members.getByRole("listitem").filter({ hasText: "Bob" }).getByRole("button", { name: "Ban" }),
  ).toHaveCount(0);
  await helper.getByRole("button", { name: "Ban" }).click();
  const ban = page.getByRole("alertdialog", { name: /Ban Helper/ });
  await expect(ban).toBeVisible();
  await expect(ban.getByRole("button", { name: /delete their messages/i })).toHaveCount(0);
  await ban.getByRole("textbox", { name: "Reason" }).fill("Spam");
  await ban.getByRole("button", { name: "For" }).click();
  await page.getByRole("option", { name: "A day" }).click();
  await ban.getByRole("button", { name: "Ban", exact: true }).click();
  await expect(ban).toBeHidden();
  await expect(page.getByText("Banned Helper")).toBeVisible();
  await expect(helper).toHaveCount(0);
  const banned = dialog.getByRole("region", { name: "Banned" });
  const row = banned.getByRole("listitem").filter({ hasText: "Helper" });
  await expect(row).toBeVisible();
  await expect(row).toContainText("Reason: Spam");
  await expect(row).toContainText("Until");
  await row.getByRole("button", { name: "Lift ban" }).click();
  await expect(row).toHaveCount(0);
  await expect(dialog.getByText("No one is banned.")).toBeVisible();
});
