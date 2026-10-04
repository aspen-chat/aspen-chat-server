import { expect, test, type Page } from "@playwright/test";
import { bob, community, helper, me, organiserRoleWith, signInToWorld } from "./world";

/**
 * Nicknames in the stubbed world's Family community, whose owner is Bob and where the caller
 * organises: a nickname names its member everywhere in the community, the caller chooses their
 * own on their card, and those who may clear the nicknames of members below them. Requests are
 * checked here, and their events published as the server would.
 */

const categories = [
  {
    id: "0190f0a0-0000-7000-8000-00000000c001",
    builtin: "spam",
    name: "Spam",
    description: "Unwanted advertising",
    hidden: false,
  },
];

/**
 * Opens #general with its member list, and waits for the community's members to be read, so the
 * events a test publishes come after the read rather than being replaced by it.
 */
async function showMembers(page: Page) {
  await page.getByText("general", { exact: true }).click();
  const toggle = page.getByRole("button", { name: "Show members" });
  if (await toggle.isVisible()) {
    await toggle.click();
  }
  await expect(
    page
      .getByRole("complementary", { name: "Members" })
      .getByRole("button", { name: "Show profile of Kate" }),
  ).toBeVisible();
}

/** Opens someone's card from the member list. */
async function openCard(page: Page, name: string) {
  await page
    .getByRole("complementary", { name: "Members" })
    .getByRole("button", { name: `Show profile of ${name}` })
    .click();
  return page.getByRole("dialog", { name: `Profile of ${name}` });
}

function nicknamed(user: string, nickname: string | null): Record<string, unknown> {
  return { serverEvent: "userCommunity", type: "update", community, user, nickname };
}

test("a nickname names its member in the community, and their card says who they are elsewhere", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  const publish = await signInToWorld(page);
  await showMembers(page);
  publish(nicknamed(bob, "Bobby"));
  const card = await openCard(page, "Bobby");
  await expect(card.getByText("Bob With A Rather Long Display Name")).toBeVisible();
  await expect(card.getByText("@bob")).toBeVisible();
  await page.keyboard.press("Escape");
  // His messages are his nickname's too.
  await expect(
    page.getByRole("main").getByRole("button", { name: "Show profile of Bobby" }).first(),
  ).toBeVisible();
  publish(nicknamed(bob, null));
  await expect(
    page
      .getByRole("complementary", { name: "Members" })
      .getByRole("button", { name: "Show profile of Bob With A Rather Long Display Name" }),
  ).toBeVisible();
});

test("the caller chooses their own nickname on their card, with Change nickname", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  const sent: unknown[] = [];
  const publish = await signInToWorld(page, async (p) => {
    await p.route(new RegExp(`/communities/${community}/members/@me$`), (route) => {
      if (route.request().method() !== "PATCH") {
        return route.fallback();
      }
      const body = route.request().postDataJSON() as { nickname?: string | null };
      sent.push(body);
      publish(nicknamed(me, body.nickname ?? null));
      return route.fulfill({
        json: { community, user: me, sortIndex: 0, roles: [], nickname: body.nickname },
      });
    });
  });
  await showMembers(page);
  // Without Change nickname, and with none to clear, the card offers nothing.
  let card = await openCard(page, "Kate");
  await expect(card.getByLabel("Your nickname in Family")).toHaveCount(0);
  await page.keyboard.press("Escape");

  publish(organiserRoleWith(["changeNickname"]));
  card = await openCard(page, "Kate");
  await card.getByLabel("Your nickname in Family").fill("Katie");
  await card.getByRole("button", { name: "Save nickname" }).click();
  expect(sent).toEqual([{ nickname: "Katie" }]);
  await expect(page.getByRole("dialog", { name: "Profile of Katie" })).toBeVisible();
  await page
    .getByRole("dialog", { name: "Profile of Katie" })
    .getByRole("button", { name: "Clear nickname" })
    .click();
  expect(sent).toEqual([{ nickname: "Katie" }, { nickname: null }]);
  await expect(page.getByRole("dialog", { name: "Profile of Kate" })).toBeVisible();
});

test("Manage nicknames clears the nickname of a member below the caller, never the owner's, and anyone reports one", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  const cleared: string[] = [];
  const reported: unknown[] = [];
  const publish = await signInToWorld(page, async (p) => {
    await p.route(new RegExp(`/communities/${community}/members/([^/]+)/nickname$`), (route) => {
      const user = route.request().url().split("/").at(-2) ?? "";
      cleared.push(user);
      publish(nicknamed(user, null));
      return route.fulfill({ status: 204 });
    });
    await p.route(/\/api\/v1\/report-categories$/, (route) => route.fulfill({ json: categories }));
    await p.route(
      new RegExp(`/communities/${community}/members/[^/]+/nickname/reports$`),
      (route) => {
        reported.push({
          url: new URL(route.request().url()).pathname,
          body: route.request().postDataJSON() as unknown,
        });
        return route.fulfill({
          status: 201,
          json: { id: "0190f0a0-0000-7000-8000-00000000c0ff", createdAt: new Date().toISOString() },
        });
      },
    );
  });
  await showMembers(page);
  publish(organiserRoleWith(["manageNicknames"]));
  publish(nicknamed(helper, "Tinker"));
  publish(nicknamed(bob, "Bobby"));

  // The owner ranks above everyone; their nickname is theirs alone.
  let card = await openCard(page, "Bobby");
  await expect(card.getByRole("button", { name: "Clear Bobby's nickname" })).toHaveCount(0);
  await card.getByRole("button", { name: "Report the nickname Bobby" }).click();
  const dialog = page.getByRole("dialog", { name: "Report the nickname “Bobby”" });
  await dialog.getByRole("button", { name: /Why are you reporting it/ }).click();
  await page.getByRole("option", { name: /Spam/ }).click();
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(page.getByText("Report sent. Thank you.")).toBeVisible();
  await expect(dialog).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(card).toHaveCount(0);
  expect(reported).toEqual([
    {
      url: `/api/v1/communities/${community}/members/${bob}/nickname/reports`,
      body: { category: categories[0]?.id },
    },
  ]);

  // The bot holds only everyone's role, below the caller's.
  card = await openCard(page, "Tinker");
  await card.getByRole("button", { name: "Clear Tinker's nickname" }).click();
  expect(cleared).toEqual([helper]);
  await expect(page.getByRole("dialog", { name: "Profile of Helper" })).toBeVisible();
});
