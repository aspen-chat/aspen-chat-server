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
  "Privacy Policy and Terms of Use": "Settings",
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

test("the fleet shows what NATS's memory holds, and says when the events stream is nearly full", async ({
  page,
}) => {
  const fleet = await openSection(page, "Server fleet");
  await expect(fleet).toContainText(
    "327 MiB of 327 MiB reserved by every stream on the NATS server, holding 245 MiB.",
  );
  const streams = fleet.getByRole("table", { name: "Streams" });
  const events = streams.getByRole("row").filter({ hasText: "Events" });
  await expect(events).toContainText("245 MiB (93%)");
  await expect(events).toContainText("263 MiB, automatic");
  const reports = streams.getByRole("row").filter({ hasText: "Voice reports" });
  await expect(reports).toContainText("64 MiB, set");
  await expect(reports).toContainText("Empty");
  await expect(fleet).toContainText("The events stream is nearly full.");
});

test("a stream's size is set in MiB, and emptied to make it automatic", async ({ page }) => {
  const policies = await openSection(page, "Policies");
  const events = policies.getByRole("textbox", { name: "MiB for the last minute of events" });
  const reports = policies.getByRole("textbox", { name: "MiB for voice servers' reports" });
  await expect(events).toHaveValue("");
  await expect(reports).toHaveValue("64");
  await events.fill("512");
  await reports.fill("");
  const saved = page.waitForRequest(
    (request) => request.method() === "PATCH" && request.url().endsWith("/api/v1/admin/settings"),
  );
  await policies.getByRole("button", { name: "Save" }).click();
  expect((await saved).postDataJSON()).toEqual({ eventStreamMib: 512, voiceReportStreamMib: null });
  await expect(policies.getByRole("status")).toHaveText("Saved.");
});

test("the fleet names a voice server reporting under an unregistered id, and shows the ids to copy", async ({
  page,
}) => {
  const fleet = await openSection(page, "Server fleet");
  await expect(
    fleet.getByText("A voice server is reporting as 0190f0a0-0000-7000-8000-000000000099"),
  ).toBeVisible();
  const voice = fleet.getByRole("table", { name: "Voice servers" });
  await expect(voice.getByRole("columnheader", { name: "ID" })).toBeVisible();
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

test("the fleet charts the database's latency by percentile, with gaps where none was recorded", async ({
  page,
}) => {
  const fleet = await openSection(page, "Server fleet");
  const statements = fleet.getByRole("group", { name: "Statements over 1 hour" });
  await expect(statements).toBeVisible();
  // Three lines, named by a legend.
  const figure = fleet.getByRole("figure").filter({ hasText: "From a server sending" });
  for (const name of ["Median", "95th percentile", "99th percentile"]) {
    await expect(figure.getByRole("listitem").filter({ hasText: name })).toBeVisible();
  }
  await statements.focus();
  const readout = statements.getByRole("status");
  await expect(readout).toContainText("800μs");
  await expect(readout).toContainText("2.4ms");
  await expect(readout).toContainText("6ms");
  // No wait was recorded in the last steps: the readout says so rather than drawing a value.
  const waits = fleet.getByRole("group", { name: "Waiting for a connection over 1 hour" });
  await waits.focus();
  await expect(waits.getByRole("status")).toContainText("—");
  await page.keyboard.press("Home");
  await expect(waits.getByRole("status")).toContainText("350μs");
  await expect(waits.getByRole("status")).toContainText("1.5s");
  await fleet.getByRole("radio", { name: "7 days" }).click();
  await expect(fleet.getByRole("group", { name: "Statements over 7 days" })).toBeVisible();
  await fleet.getByRole("button", { name: "Show as table" }).click();
  const table = fleet.getByRole("table", { name: "Waiting for a connection" });
  // Only the steps with something recorded, under the heading row.
  await expect(table.getByRole("row")).toHaveCount(159);
  await expect(table.getByRole("row").nth(1)).toContainText("400");
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

test("a voice server is registered from the fleet tab, which shows what to give it", async ({
  page,
}) => {
  await openSection(page, "Server fleet");
  const servers = page.getByRole("region", { name: "Voice servers" });
  await expect(servers.getByText("voice-east", { exact: true })).toBeVisible();
  await servers.getByRole("button", { name: "Add a voice server" }).click();
  await servers.getByRole("textbox", { name: "Name" }).fill("voice-north");
  await servers.getByRole("textbox", { name: "Address" }).fill("https://voice-north.example");
  await servers.getByRole("textbox", { name: "Capacity" }).fill("250");
  const created = page.waitForRequest(
    (r) => r.method() === "POST" && r.url().endsWith("/api/v1/voice-servers"),
  );
  await servers.getByRole("button", { name: "Add a voice server" }).click();
  expect((await created).postDataJSON()).toEqual({
    name: "voice-north",
    url: "https://voice-north.example",
    capacity: 250,
  });
  // The one just added shows its id and NATS user at once.
  await expect(servers.getByText('id = "0190f0a0-0000-7000-8000-000000000041"')).toBeVisible();
  await expect(servers.getByText('user: "voice-north"')).toBeVisible();
});

test("a user's name in the directory opens their card, with what may be done to them", async ({
  page,
}) => {
  const users = await openSection(page, "Users");
  await users
    .getByRole("button", { name: "Open Bob With A Rather Long Display Name's profile" })
    .click();
  const card = page.getByRole("dialog", { name: /Bob With A Rather Long Display Name/ });
  await expect(card.getByText("On this server")).toBeVisible();
  await expect(card.getByRole("button", { name: "Message" })).toBeVisible();
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

test("a Privacy Policy is published, read on its page, and withdrawn", async ({ page }) => {
  const documents = await openSection(page, "Privacy Policy and Terms of Use");
  const policy = documents.locator("form").first();
  const field = policy.getByRole("textbox", { name: "Privacy Policy" });
  await expect(policy.getByText(/^Not published\./)).toBeVisible();
  const publish = policy.getByRole("button", { name: "Publish" });
  await expect(publish).toBeDisabled();
  await field.fill("# Ours\n\nWe keep little.");
  await publish.click();
  await expect(policy.getByRole("status")).toHaveText("Published.");
  await expect(publish).toBeDisabled();

  await policy.getByRole("link", { name: "Open its page" }).click();
  await expect(page).toHaveURL(/\/privacy$/);
  await expect(page.getByText("We keep little.")).toBeVisible();
  await page.goBack();

  const withdraw = policy.getByRole("button", { name: "Withdraw" });
  await withdraw.click();
  await expect(policy.getByRole("status")).toHaveText(/^Withdrawn\./);
  await expect(withdraw).toHaveCount(0);
  // What was written stays, to publish again.
  await expect(field).toHaveValue("# Ours\n\nWe keep little.");
  await expect(publish).toBeEnabled();
});
