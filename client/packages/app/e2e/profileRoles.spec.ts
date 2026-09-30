import { expect, test, type Page } from "@playwright/test";
import { community, helper, signInToWorld } from "./world";

/**
 * Roles on a user card, against the stubbed world in `world.ts`, where the caller organises the
 * Family community with the Organiser role, and may give and take roles below it.
 */

const organiserRole = "0190f0a0-0000-7000-8000-000000000041";
const greeter = "0190f0a0-0000-7000-8000-000000000042";

/** Opens someone's card from the member list, going back to the channel on a phone first. */
async function openCard(page: Page, name: string) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText("general", { exact: true }).click();
  const toggle = page.getByRole("button", { name: "Show members" });
  if (await toggle.isVisible()) {
    await toggle.click();
  }
  await page
    .getByRole("complementary", { name: "Members" })
    .getByRole("button", { name: `Show profile of ${name}` })
    .click();
  return page.getByRole("dialog", { name: `Profile of ${name}` });
}

test("a card opened in a community lists the member's roles, and gives and takes them for those who may", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  const given: string[] = [];
  const publish = await signInToWorld(page, async (p) => {
    await p.route(
      new RegExp(`/communities/${community}/members/${helper}/roles/[^/]+$`),
      (route) => {
        const method = route.request().method();
        given.push(`${method} ${route.request().url().split("/").pop() ?? ""}`);
        publish({
          serverEvent: "userCommunity",
          type: "update",
          community,
          user: helper,
          roles: method === "PUT" ? [greeter] : [],
        });
        return route.fulfill({ status: 204 });
      },
    );
  });
  await expect(page.getByText("general", { exact: true })).toBeVisible();

  // With no role below the caller's own, there is nothing to give.
  let card = await openCard(page, "Helper");
  let roles = card.getByRole("region", { name: "Roles" });
  await expect(roles.getByText("No roles here yet.")).toBeVisible();
  await expect(roles.getByRole("button")).toHaveCount(0);
  await page.keyboard.press("Escape");

  // A new role below the Organiser's can be given.
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
  });
  card = await openCard(page, "Helper");
  roles = card.getByRole("region", { name: "Roles" });
  await roles.getByRole("button", { name: "Give Helper a role" }).click();
  const menu = page.getByRole("menu", { name: "Give Helper a role" });
  await expect(menu.getByRole("menuitem")).toHaveText(["Greeter"]);
  await menu.getByRole("menuitem", { name: "Greeter" }).click();
  await expect(roles.getByRole("listitem").filter({ hasText: "Greeter" })).toBeVisible();
  expect(given).toEqual([`PUT ${greeter}`]);

  // The same card takes it away again.
  await roles.getByRole("button", { name: "Take Greeter from Helper" }).click();
  await expect(roles.getByRole("listitem").filter({ hasText: "Greeter" })).toHaveCount(0);
  expect(given).toEqual([`PUT ${greeter}`, `DELETE ${greeter}`]);
});

test("a card opened outside a community shows no roles", async ({ page }) => {
  await signInToWorld(page);
  await page
    .getByRole("navigation", { name: "Communities" })
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await page.getByRole("link", { name: /Bob/ }).first().click();
  await page
    .getByRole("button", { name: /^Show profile of Bob/ })
    .first()
    .click();
  const card = page.getByRole("dialog", { name: /^Profile of Bob/ });
  await expect(card).toBeVisible();
  await expect(card.getByRole("region", { name: "Roles" })).toHaveCount(0);
});
