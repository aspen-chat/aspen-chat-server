import { expect, test } from "@playwright/test";
import { community, settleAnimations, signInToWorld } from "./world";

const organiserRole = "0190f0a0-0000-7000-8000-000000000041";
const greeter = "0190f0a0-0000-7000-8000-000000000042";

/**
 * The community's settings and a channel's access settings, as the organiser of the stubbed
 * world in `world.ts` sees them: they hold a role that manages everything but other people's
 * messages, below Bob, the owner.
 */

test("community settings list the roles, highest first, and the members with theirs", async ({
  page,
}) => {
  await signInToWorld(page);
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
  // Only Helper, a bot ranked below them, may be removed; never the owner.
  await expect(dialog.getByRole("button", { name: "Remove" })).toHaveCount(1);
  await expect(
    dialog
      .getByRole("listitem")
      .filter({ hasText: "Helper" })
      .getByRole("button", { name: "Remove" }),
  ).toHaveCount(1);
  // They may assign roles, so they search every member, not only the sample.
  await dialog.getByRole("searchbox", { name: "Find a member" }).fill("kate");
  await expect(dialog.getByText("Search every member by name.")).toBeVisible();
  await expect(dialog.getByRole("listitem")).toHaveCount(1);
});

test("a channel's access starts open to everyone and explains what a member can do", async ({
  page,
}) => {
  await signInToWorld(page);
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
  // The list of members rises into place; a press on an option still moving can miss it.
  const option = page.getByRole("option").first();
  await expect(option).toBeVisible();
  await settleAnimations(page);
  await option.click();
  await expect(dialog.getByRole("row", { name: /View channels/ })).toContainText("Yes", {
    timeout: 10_000,
  });
});

test("a role's colour and showing it apart are chosen, previewed, and saved together", async ({
  page,
}) => {
  const patches: unknown[] = [];
  const publish = await signInToWorld(page, async (p) => {
    await p.route(new RegExp(`/api/v1/roles/${greeter}$`), (route) => {
      const patch = route.request().postDataJSON() as Record<string, unknown>;
      patches.push(patch);
      publish({ serverEvent: "role", type: "update", id: greeter, ...patch });
      return route.fulfill({ status: 200, json: {} });
    });
  });
  await expect(page.getByText("general", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Community settings" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("tab", { name: "Roles" }).click();
  const roles = dialog.getByRole("grid", { name: "Roles, highest first" }).getByRole("row");
  await expect(roles).toHaveText(["Organiser", "everyone"]);
  // A role below the organiser's, which they may change.
  publish({ serverEvent: "role", type: "update", id: organiserRole, position: 2 });
  publish({
    serverEvent: "role",
    type: "create",
    id: greeter,
    community,
    name: "Greeter",
    position: 1,
    permissions: [],
    everyone: false,
    hoist: false,
  });
  await expect(roles).toHaveText(["Organiser", "Greeter", "everyone"]);
  // Everyone's role is never coloured or shown apart.
  await roles.last().click();
  await expect(dialog.getByRole("checkbox", { name: /^Colour names/ })).toHaveCount(0);
  await roles.filter({ hasText: "Greeter" }).click();
  await dialog.getByText("Colour names", { exact: true }).click();
  await expect(dialog.getByRole("checkbox", { name: /^Colour names/ })).toBeChecked();
  const hue = dialog.getByRole("slider", { name: "Hue" });
  await hue.focus();
  for (let i = 0; i < 5; i++) {
    await page.keyboard.press("ArrowRight");
  }
  await expect(hue).toHaveValue("215");
  // The name is previewed on a light ground and a dark one, besides its row in the list.
  await expect(dialog.getByText("Greeter", { exact: true })).toHaveCount(3);
  await dialog.getByText("Show members separately", { exact: true }).click();
  await dialog.getByRole("button", { name: "Save role" }).click();
  await expect.poll(() => patches).toEqual([{ hue: 215, hoist: true }]);
  await expect(dialog.getByText("Unsaved changes")).toHaveCount(0);
});

test("the member list puts online holders of a role shown apart under it, in its colour", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list beside it");
  await signInToWorld(page);
  await page.getByText("general", { exact: true }).click();
  const members = page.getByRole("complementary", { name: "Members" });
  await expect(members.getByRole("heading", { name: "Organiser — 1" })).toBeVisible();
  const kate = members.getByRole("button", { name: "Show profile of Kate" });
  await expect(kate.getByText("Kate", { exact: true })).toHaveAttribute("style", /light-dark/);
  // Nobody else holds it, so they are listed as they were.
  await expect(members.getByRole("heading", { name: /^Online — / })).toBeVisible();
});
