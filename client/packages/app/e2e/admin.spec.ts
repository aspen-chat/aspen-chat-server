import { expect, test, type Page } from "@playwright/test";
import { signInToWorld, standingInvite } from "./world";

/**
 * The Administration Dashboard, against the stubbed world in `world.ts`, where the caller
 * administers a server that takes accounts by invite, with two API servers and three voice
 * servers.
 */

async function openDashboard(page: Page) {
  const rail = page.getByRole("navigation", { name: "Communities" });
  await rail.getByRole("link", { name: "Administration" }).click();
  await expect(page.getByRole("heading", { name: "Administration", level: 1 })).toBeVisible();
}

const section = (page: Page, name: string) => page.getByRole("region", { name });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
  await openDashboard(page);
});

test("the overview counts users and communities and says how people join", async ({ page }) => {
  const overview = section(page, "Overview");
  await expect(overview.getByText("+2 this week")).toBeVisible();
  await expect(overview.getByText("Invite only")).toBeVisible();
});

test("the fleet names each server's state in words", async ({ page }) => {
  const fleet = section(page, "Server fleet");
  const api = fleet.getByRole("table", { name: "API servers" });
  await expect(api.getByRole("row").filter({ hasText: "api-1" })).toContainText("Healthy");
  await expect(api.getByRole("row").filter({ hasText: "api-2" })).toContainText("Errors");
  await expect(api.getByRole("row").filter({ hasText: "api-1" })).toContainText("214 MiB");
  const voice = fleet.getByRole("table", { name: "Voice servers" });
  await expect(voice.getByRole("row").filter({ hasText: "voice-east" })).toContainText("Reporting");
  await expect(voice.getByRole("row").filter({ hasText: "voice-west" })).toContainText("Silent");
  await expect(voice.getByRole("row").filter({ hasText: "voice-spare" })).toContainText("Disabled");
});

test("an administrator makes an invite, copies it, and revokes it", async ({ page }) => {
  const invites = section(page, "Registration invites");
  await expect(invites.getByRole("row").filter({ hasText: standingInvite })).toContainText(
    "1 of 2",
  );
  await invites.getByRole("textbox", { name: "Uses" }).fill("3");
  await invites.getByRole("textbox", { name: "Note" }).fill("for grandma");
  await invites.getByRole("button", { name: "Create invite" }).click();
  const made = invites.getByRole("row").filter({ hasText: "for grandma" });
  await expect(made).toContainText("0 of 3");
  await expect(made).toContainText("Usable");
  await made.getByRole("button", { name: "Copy link" }).click();
  await expect(made.getByRole("button", { name: "Copied" })).toBeVisible();
  await made.getByRole("button", { name: /^Revoke invite / }).click();
  const confirm = page.getByRole("alertdialog", { name: "Revoke this invite?" });
  await confirm.getByRole("button", { name: "Revoke", exact: true }).click();
  await expect(made).toContainText("Revoked");
  await expect(made.getByRole("button", { name: /^Revoke invite / })).toHaveCount(0);
});

test("the growth charts follow the range buttons, and read out on hover and keys", async ({
  page,
}) => {
  const growth = section(page, "Growth");
  const users = growth.getByRole("group", { name: "Users over 3 months" });
  await expect(users).toBeVisible();
  await expect(growth.getByRole("group", { name: "Communities over 3 months" })).toBeVisible();
  // The latest value is labelled at the line's end.
  await expect(users.locator("svg")).toContainText("416");
  await users.focus();
  await expect(users.getByRole("status")).toContainText("416");
  await page.keyboard.press("Home");
  await expect(users.getByRole("status")).toContainText("380");
  await growth.getByRole("radio", { name: "5 years" }).click();
  await expect(growth.getByRole("group", { name: "Users over 5 years" })).toBeVisible();
  await growth.getByRole("button", { name: "Show as table" }).click();
  const table = growth.getByRole("table", { name: "Growth" });
  await expect(table.getByRole("row")).toHaveCount(62);
  await expect(table.getByRole("row").nth(1)).toContainText("364");
});

test("users and communities are searched, sorted, and paged", async ({ page }) => {
  const users = section(page, "Users");
  const rows = users.getByRole("row");
  // Fifteen to a page, newest first.
  await expect(rows).toHaveCount(16);
  await expect(users.getByText("1–15")).toBeVisible();
  await users.getByRole("button", { name: "Next" }).click();
  await expect(rows).toHaveCount(8);
  await expect(users.getByText("16–22")).toBeVisible();
  await expect(users.getByRole("button", { name: "Next" })).toBeDisabled();
  await users.getByRole("button", { name: "Previous" }).click();
  await expect(users.getByText("1–15")).toBeVisible();
  // Sorting by name starts at A and returns to the first page.
  await users.getByRole("columnheader", { name: "Name" }).getByRole("button").click();
  await expect(users.getByRole("columnheader", { name: "Name" })).toHaveAttribute(
    "aria-sort",
    "ascending",
  );
  await expect(rows.nth(1)).toContainText("Bob With A Rather Long Display Name");
  await users.getByRole("columnheader", { name: "Name" }).getByRole("button").click();
  await expect(rows.nth(1)).toContainText("Member 20");
  await expect(users.getByRole("columnheader", { name: "Invite" }).getByRole("button")).toHaveCount(
    0,
  );
  // A larger page holds everyone.
  await users.getByRole("button", { name: /Rows per page/ }).click();
  await page.getByRole("option", { name: "30" }).click();
  await expect(rows).toHaveCount(23);
  await users.getByRole("searchbox", { name: "Search users" }).fill("rather long");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1)).toContainText(standingInvite);
  await users.getByRole("searchbox", { name: "Search users" }).fill("nobody");
  await expect(users.getByText("Nothing matches.")).toBeVisible();
  const communities = section(page, "Communities");
  await communities.getByRole("columnheader", { name: "Members" }).getByRole("button").click();
  await expect(communities.getByRole("row").nth(1)).toContainText("Book club");
});

test("nothing on the dashboard makes the screen scroll sideways", async ({ page }) => {
  await expect(
    section(page, "Server fleet").getByText("voice-east", { exact: true }),
  ).toBeVisible();
  const widths = await page.evaluate(() => ({
    page: document.documentElement.scrollWidth,
    screen: window.innerWidth,
  }));
  expect(widths.page).toBeLessThanOrEqual(widths.screen);
});

test("deployment roles list, and the caller's own top role is theirs but not theirs to change", async ({
  page,
}) => {
  const roles = section(page, "Deployment roles");
  await expect(roles.getByRole("row", { name: "Administrator" })).toBeVisible();
  await expect(roles.getByText("This role is at or above your highest role")).toBeVisible();
  // Moderation is not the Administrator's, so it cannot be given from here.
  await expect(roles.getByRole("checkbox", { name: /Moderate any community/ })).not.toBeChecked();
  const users = section(page, "Users");
  await expect(
    users.getByRole("row").filter({ hasText: "Kate" }).getByText("Administrator"),
  ).toBeVisible();
  await expect(section(page, "Moderation log").getByText("Nothing yet.")).toBeVisible();
});
