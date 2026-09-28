import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * The community's settings and a channel's access settings, as the organiser of the stubbed
 * world in `world.ts` sees them: they hold a role that manages everything but other people's
 * messages, below Bob, the owner.
 */

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("community settings list the roles, highest first, and the members with theirs", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Community settings" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("tab", { name: "Roles" }).click();
  const roles = dialog.getByRole("grid", { name: "Roles, highest first" }).getByRole("row");
  await expect(roles).toHaveText(["Organiser", "everyone"]);
  // The organiser's own role ranks with them, so they may not change it.
  await roles.first().click();
  await expect(dialog.getByText("This role is at or above your highest role")).toBeVisible();
  await roles.last().click();
  await expect(dialog.getByRole("checkbox", { name: /^Send messages Post/ })).toBeChecked();
  // Managing other people's messages is not theirs to give.
  await expect(dialog.getByRole("checkbox", { name: /Manage messages/ })).toBeDisabled();
  await dialog.getByRole("tab", { name: "Members" }).click();
  await expect(dialog.getByText("Owner")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Remove" })).toHaveCount(0);
  // They may assign roles, so they search every member, not only the sample.
  await dialog.getByRole("searchbox", { name: "Find a member" }).fill("kate");
  await expect(dialog.getByText("Search every member by name.")).toBeVisible();
  await expect(dialog.getByRole("listitem")).toHaveCount(1);
});

test("a channel's access starts open to everyone and explains what a member can do", async ({
  page,
}) => {
  const general = page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByRole("row", {
      name: /general/,
    });
  await general.getByRole("button", { name: /Options for general/ }).click();
  await page.getByRole("menuitem", { name: "Who can use this" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Who can use #general" })).toBeVisible();
  await expect(dialog.getByRole("radio", { name: /^Everyone Every member/ })).toBeChecked();
  await dialog.getByText("Check access").click();
  await dialog.getByRole("combobox", { name: "Member" }).click();
  await page.getByRole("option").first().click();
  await expect(dialog.getByRole("row", { name: /View channels/ })).toContainText("Yes");
});
