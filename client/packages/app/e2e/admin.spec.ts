import { expect, test, type Page } from "@playwright/test";
import { deploymentAdministrator } from "./world/fixtures";
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

/** The tab each section of the dashboard is on. */
const TAB_OF: Record<string, string> = {
  Overview: "Overview",
  Growth: "Overview",
  "Server fleet": "Server fleet",
  "Registration invites": "Registration invites",
  "Deployment roles": "Deployment roles",
  Users: "Users",
  Communities: "Communities",
  "Moderation log": "Moderation log",
  "Deployment profile": "Settings",
  Policies: "Settings",
};

const tabs = (page: Page) => page.getByRole("navigation", { name: "Administration sections" });

/** Opens the tab a section is on, and finds the section there. */
async function openSection(page: Page, name: string) {
  await tabs(page)
    .getByRole("link", { name: TAB_OF[name] ?? name, exact: true })
    .click();
  return page.getByRole("region", { name });
}

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
  await openDashboard(page);
});

test("the overview counts users and communities and says how people join", async ({ page }) => {
  const overview = await openSection(page, "Overview");
  await expect(overview.getByText("+2 this week")).toBeVisible();
  await expect(overview.getByText("Invite only")).toBeVisible();
});

test("the fleet names each server's state in words", async ({ page }) => {
  const fleet = await openSection(page, "Server fleet");
  const api = fleet.getByRole("table", { name: "API servers" });
  await expect(api.getByRole("row").filter({ hasText: "api-1" })).toContainText("Healthy");
  await expect(api.getByRole("row").filter({ hasText: "api-2" })).toContainText("Errors");
  await expect(api.getByRole("row").filter({ hasText: "api-1" })).toContainText("214 MiB");
  const voice = fleet.getByRole("table", { name: "Voice servers" });
  await expect(voice.getByRole("row").filter({ hasText: "voice-east" })).toContainText("Reporting");
  await expect(voice.getByRole("row").filter({ hasText: "voice-west" })).toContainText("Silent");
  await expect(voice.getByRole("row").filter({ hasText: "voice-spare" })).toContainText("Disabled");
});

test("an administrator makes an invite, sees its QR code, copies it, and revokes it", async ({
  page,
}) => {
  const invites = await openSection(page, "Registration invites");
  await expect(invites.getByRole("row").filter({ hasText: standingInvite })).toContainText(
    "1 of 2",
  );
  await invites.getByRole("textbox", { name: "Uses" }).fill("3");
  await invites.getByRole("textbox", { name: "Note" }).fill("for grandma");
  await invites.getByRole("button", { name: "Create invite" }).click();
  // The invite just made shows its link's QR code at once.
  const shown = page.getByRole("dialog", { name: /^QR code for invite / });
  await expect(shown.getByRole("img", { name: /^QR code for invite / })).toBeVisible();
  await expect(shown).toContainText("/register?invite=");
  await expect(shown.getByRole("button", { name: "Download" })).toBeVisible();
  await shown.getByRole("button", { name: "Close" }).click();
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
  const growth = await openSection(page, "Growth");
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
  const users = await openSection(page, "Users");
  const rows = users.getByRole("row");
  // Fifteen to a page, newest first.
  await expect(rows).toHaveCount(16);
  await expect(users.getByText("1–15")).toBeVisible();
  await users.getByRole("button", { name: "Next" }).click();
  await expect(rows).toHaveCount(8);
  await expect(users.getByText("16–22")).toBeVisible();
  // Paging before the search box has ever been typed in stays put once its pause has passed.
  await page.waitForTimeout(500);
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
  const communities = await openSection(page, "Communities");
  await communities.getByRole("columnheader", { name: "Members" }).getByRole("button").click();
  await expect(communities.getByRole("row").nth(1)).toContainText("Book club");
});

test("the dashboard opens at its first tab, and no tab makes the screen scroll sideways", async ({
  page,
}) => {
  await expect(page).toHaveURL(/\/admin\/overview$/);
  await expect(tabs(page).getByRole("link", { name: "Overview" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(
    (await openSection(page, "Server fleet")).getByText("voice-east", { exact: true }),
  ).toBeVisible();
  for (const link of await tabs(page).getByRole("link").all()) {
    await link.click();
    await expect(link).toHaveAttribute("aria-current", "page");
    const widths = await page.evaluate(() => ({
      page: document.documentElement.scrollWidth,
      screen: window.innerWidth,
    }));
    expect(widths.page, await link.innerText()).toBeLessThanOrEqual(widths.screen);
  }
});

test("deployment roles list, and the caller's own top role is theirs but not theirs to change", async ({
  page,
}) => {
  const roles = await openSection(page, "Deployment roles");
  await expect(roles.getByRole("row", { name: "Administrator" })).toBeVisible();
  await expect(roles.getByText("This role is at or above your highest role")).toBeVisible();
  // Moderation is not the Administrator's, so it cannot be given from here.
  await expect(roles.getByRole("checkbox", { name: /Moderate any community/ })).not.toBeChecked();
  const users = await openSection(page, "Users");
  await expect(
    users.getByRole("row").filter({ hasText: "Kate" }).getByText("Administrator"),
  ).toBeVisible();
  await expect((await openSection(page, "Moderation log")).getByText("Nothing yet.")).toBeVisible();
});

test("a permission that another includes shows as held, and fixed, while that one is chosen", async ({
  page,
}) => {
  const json = (body: unknown) => ({ contentType: "application/json", body: JSON.stringify(body) });
  const administrator = [
    "viewDashboard",
    "manageDeploymentRoles",
    "moderateCommunities",
    "removeContent",
    "reviewReports",
  ];
  await page.route(/\/api\/v1\/users\/@me\/admin$/, (route) =>
    route.fulfill(
      json({
        permissions: administrator,
        roles: [deploymentAdministrator],
        inclusions: [{ permission: "moderateCommunities", includes: ["removeContent"] }],
      }),
    ),
  );
  await page.route(/\/api\/v1\/admin\/roles$/, (route) =>
    route.fulfill(
      json([
        {
          id: "01a0e000-0000-7000-8000-00000000a001",
          name: "Moderators",
          position: 1,
          permissions: ["reviewReports", "moderateCommunities"],
        },
        {
          id: deploymentAdministrator,
          name: "Administrator",
          position: 2,
          permissions: administrator,
        },
      ]),
    ),
  );
  // The dashboard read the roles as it opened, so it reads them again with these in place.
  await page.reload();
  const roles = await openSection(page, "Deployment roles");
  await roles.getByRole("row", { name: "Moderators" }).click();
  const remove = roles.getByRole("checkbox", { name: /^Remove content/ });
  await expect(remove).toBeChecked();
  await expect(remove).toBeDisabled();
  await expect(roles.getByText("Included in Moderate any community.")).toBeVisible();
  await roles.getByText("Moderate any community", { exact: true }).click();
  await expect(remove).not.toBeChecked();
  await expect(remove).toBeEnabled();
});

test("the deployment profile renames the server its sign-in screen welcomes people to", async ({
  page,
}) => {
  const profile = await openSection(page, "Deployment profile");
  const name = profile.getByRole("textbox", { name: "Display name" });
  await expect(name).toHaveValue("Family Server");
  const save = profile.getByRole("button", { name: "Save" });
  // Nothing to save until the name changes.
  await expect(save).toBeDisabled();
  await name.fill("  Kiesel Family Chat ");
  await save.click();
  await expect(profile.getByRole("status")).toHaveText("Saved.");
  await expect(save).toBeDisabled();
  // Emptied, the name is cleared rather than saved blank.
  const saved = page.waitForRequest(
    (request) => request.method() === "PATCH" && request.url().endsWith("/api/v1/deployment"),
  );
  await name.fill("");
  await save.click();
  expect((await saved).postDataJSON()).toEqual({ displayName: null });
  await expect(profile.getByRole("button", { name: "Add icon" })).toBeVisible();
});

test("the policies send only what changed, and reach the server at once", async ({ page }) => {
  const policies = await openSection(page, "Policies");
  const twoFactor = policies.getByRole("checkbox", { name: /Every account needs a second factor/ });
  await expect(twoFactor).not.toBeChecked();
  const save = policies.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await policies.getByText("Every account needs a second factor").click();
  await policies.getByRole("textbox", { name: "Bots per person" }).fill("5");
  const saved = page.waitForRequest(
    (request) => request.method() === "PATCH" && request.url().endsWith("/api/v1/admin/settings"),
  );
  await save.click();
  expect((await saved).postDataJSON()).toEqual({ requireTwoFactor: true, botsMaxPerUser: 5 });
  await expect(policies.getByRole("status")).toHaveText("Saved.");
  await expect(twoFactor).toBeChecked();
  await expect(save).toBeDisabled();
});

test("the plugins tab lists what is installed, what each may do, and what it keeps", async ({
  page,
}) => {
  const section = await openSection(page, "Plugins");
  await expect(section.getByRole("heading", { name: /Word filter/ })).toBeVisible();
  await expect(section).toContainText("Change what people write before it is sent");
  await expect(section).toContainText("Keeps a count of each channel's messages.");
  // Without Manage plugins, nothing here changes anything.
  await expect(section.getByRole("button", { name: "Turn off" })).toHaveCount(0);
});
