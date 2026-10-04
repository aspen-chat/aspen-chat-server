import { expect, test, type Page } from "@playwright/test";
import {
  foreignCommunity,
  foreignDomain,
  foreignInviteCode,
  stubForeignDeployment,
} from "./foreignWorld";
import { inviteCode, signInToWorld } from "./world";

/**
 * Other deployments beside the home, against the stubbed world in `world.ts` and a stubbed
 * `beta.example` (`foreignWorld.ts`): the rail and the DM list mix them, each foreign entry
 * marked with its domain, and their routes live under `/at/{domain}`.
 */

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test("a deployment the home lists joins the rail and the DM list", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  const entry = rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` });
  await expect(entry).toBeVisible();
  await expect(rail(page).getByRole("row", { name: /^Family/ })).toBeVisible();
  await entry.click();
  await expect(page).toHaveURL(new RegExp(`/at/beta\\.example/communities/${foreignCommunity}`));
  await expect(page.getByRole("heading", { name: "Beta club" })).toBeVisible();

  await rail(page)
    .getByRole("link", { name: /Direct messages/ })
    .click();
  const dms = page.getByRole("navigation", { name: "Direct messages" });
  const foreignRow = dms.getByRole("link", { name: /Hostess/ });
  await expect(foreignRow).toContainText(foreignDomain);
  await expect(dms.getByRole("link").first()).toContainText("Hostess");
  await foreignRow.click();
  await expect(page).toHaveURL(/\/at\/beta\.example\/dms\//);
});

test("someone adds another server from the rail", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: false }));
  await expect(rail(page).getByRole("row", { name: /^Family/ })).toBeVisible();
  await expect(rail(page).getByRole("row", { name: /Beta club/ })).toHaveCount(0);
  await rail(page).getByRole("button", { name: "Create or join a community" }).click();
  await page.getByRole("button", { name: /Use another server/ }).click();
  await page.getByRole("textbox", { name: "Server" }).fill(foreignDomain);
  await page.getByRole("dialog").getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/at\/beta\.example\/communities\//);
});

test("a new message starts on the deployment chosen", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  await expect(
    rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` }),
  ).toBeVisible();
  await rail(page)
    .getByRole("link", { name: /Direct messages/ })
    .click();
  await page.getByRole("button", { name: "New message" }).click();
  const picker = page.getByRole("dialog", { name: "New message" });
  await picker.getByRole("radio", { name: foreignDomain }).check({ force: true });
  await picker.getByRole("option", { name: /Hostess/ }).click();
  await picker.getByRole("button", { name: "Message", exact: true }).click();
  await expect(page).toHaveURL(/\/at\/beta\.example\/dms\//);
});

test("a moderator bans a user of another server, and lifts the ban", async ({ page }) => {
  const stranger = "0290f0a0-0000-7000-8000-0000000000aa";
  const bans: { method: string; body: unknown }[] = [];
  let banned: { bannedAt: string; reason: string | null; until: string | null } | null = null;
  await signInToWorld(page, async (p) => {
    const json = (body: unknown, status = 200) => ({
      status,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
    await p.route(/\/api\/v1\/users\/@me\/admin$/, (route) =>
      route.fulfill(
        json({
          permissions: ["viewDashboard", "banUsers"],
          roles: [],
          inclusions: [{ permission: "moderateCommunities", includes: ["removeContent"] }],
        }),
      ),
    );
    await p.route(/\/api\/v1\/admin\/users(\?.*)?$/, (route) =>
      route.fulfill(
        json([
          {
            id: stranger,
            name: "stranger",
            displayName: "Stranger",
            icon: null,
            createdAt: new Date().toISOString(),
            roles: [],
            registeredWith: null,
            bot: false,
            botOwner: null,
            homeDomain: foreignDomain,
            system: false,
            banned: banned !== null,
            ban: banned,
          },
        ]),
      ),
    );
    await p.route(/\/api\/v1\/admin\/users\/[^/]+\/ban$/, (route) => {
      const method = route.request().method();
      bans.push({ method, body: method === "PUT" ? route.request().postDataJSON() : null });
      if (method === "PUT") {
        banned = { bannedAt: new Date().toISOString(), reason: "Spam", until: null };
        return route.fulfill(json({ banned: [stranger], deletedMessages: 0 }, 201));
      }
      banned = null;
      return route.fulfill({ status: 204 });
    });
  });
  await rail(page).getByRole("link", { name: "Administration" }).click();
  await page
    .getByRole("navigation", { name: "Administration sections" })
    .getByRole("link", { name: "Users" })
    .click();
  const users = page.getByRole("region", { name: "Users" });
  const row = users.getByRole("row").filter({ hasText: "Stranger" });
  await expect(row).toContainText(`stranger@${foreignDomain}`);
  await row.getByRole("button", { name: "Ban Stranger from this server" }).click();
  const dialog = page.getByRole("alertdialog", { name: "Ban Stranger from this server" });
  // Without Moderate any community, a ban deletes no messages.
  await expect(dialog.getByRole("button", { name: /delete their messages/i })).toHaveCount(0);
  await dialog.getByRole("textbox", { name: "Reason" }).fill("Spam");
  await dialog.getByRole("button", { name: "Ban", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("Banned Stranger from this server.")).toBeVisible();
  await expect(row).toContainText("Banned");
  await row.getByRole("button", { name: "Lift the ban on Stranger" }).click();
  await expect(row.getByRole("button", { name: "Ban Stranger from this server" })).toBeVisible();
  expect(bans).toEqual([
    { method: "PUT", body: { reason: "Spam", withOwner: false } },
    { method: "DELETE", body: null },
  ]);
});

test("an invite's link names its server", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  await page.evaluate(() => {
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: (text: string) => {
          document.body.dataset.copied = text;
          return Promise.resolve();
        },
      },
    });
  });
  await page.getByRole("button", { name: "Invite people" }).click();
  const invite = page.getByRole("dialog").locator("li").first();
  await invite.getByRole("button", { name: "Copy link" }).click();
  await expect(page.locator("body")).toHaveAttribute(
    "data-copied",
    new RegExp(`/invite/${inviteCode}\\?at=home\\.example$`),
  );
});

test("an invite link naming another server opens there, and one naming home stays", async ({
  page,
}) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  await expect(rail(page).getByRole("row", { name: /^Family/ })).toBeVisible();
  await page.goto(`/invite/${foreignInviteCode}?at=${foreignDomain}`);
  await expect(page).toHaveURL(new RegExp(`/at/beta\\.example/invite/${foreignInviteCode}$`));
  await expect(page.getByRole("heading", { name: /Beta club/ })).toBeVisible();

  await page.goto(`/invite/${inviteCode}?at=home.example`);
  await expect(page.getByRole("heading", { name: /Family/ })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`^[^#]*/invite/${inviteCode}\\?at=home\\.example$`));
});

test("a pasted invite link opens on the server it names", async ({ page }) => {
  await signInToWorld(page, (p) => stubForeignDeployment(p, { listed: true }));
  // The rail renders again as each deployment syncs, and a press that lands meanwhile can be
  // lost, so it is left to settle first.
  await expect(
    rail(page).getByRole("row", { name: `Beta club on ${foreignDomain}` }),
  ).toBeVisible();
  await expect(rail(page).getByRole("row", { name: /^Family/ })).toBeVisible();
  await rail(page).getByRole("button", { name: "Create or join a community" }).click();
  await page.getByRole("button", { name: /Join a community/ }).click();
  await page
    .getByRole("textbox", { name: "Invite link or code" })
    .fill(`https://elsewhere.example/invite/${foreignInviteCode}?at=${foreignDomain}`);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page).toHaveURL(new RegExp(`/at/beta\\.example/invite/${foreignInviteCode}$`));
  await expect(page.getByRole("heading", { name: /Beta club/ })).toBeVisible();
});

test("signed out, an invite link says the account can be on any server", async ({ page }) => {
  await page.goto(`/invite/${foreignInviteCode}?at=${foreignDomain}`);
  await expect(
    page.getByText(`This invite is to a community on ${foreignDomain}.`, { exact: false }),
  ).toBeVisible();
});
