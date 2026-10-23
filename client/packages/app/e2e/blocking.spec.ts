import { expect, test, type Locator, type Page } from "@playwright/test";
import { signInToWorld } from "./world";
import { user } from "./world/fixtures";

/**
 * Blocking someone from their profile card, against the stubbed world in `world.ts`, where Bob
 * writes in #general and has a DM with the caller. Runs at every size, so each step goes
 * through what a phone shows too.
 */

const bob = "Bob With A Rather Long Display Name";
const bobText = "Sounds good. See everyone at ten, and bring a jumper in case it rains.";

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

/** The card of the author of the message saying `text`. */
async function authorCard(page: Page, text: string): Promise<Locator> {
  await page
    .locator("article")
    .filter({ hasText: text })
    .getByRole("button", { name: `Show profile of ${bob}` })
    .first()
    .click();
  return page.getByRole("dialog", { name: new RegExp(bob) });
}

/** Blocks Bob from the card of one of his messages, which collapses as he is blocked. */
async function blockBob(page: Page, text: string) {
  const card = await authorCard(page, text);
  await card.getByRole("button", { name: "Block" }).click();
  // The first press explains what blocking does; the second blocks.
  await expect(card.getByText(`Block ${bob}?`)).toBeVisible();
  await card.getByRole("button", { name: "Block" }).click();
  await expect(page.locator("article").filter({ hasText: text })).toHaveCount(0);
}

test("a blocked user's messages collapse, open on request, and return when unblocked", async ({
  page,
}) => {
  await page.getByText("general", { exact: true }).click();
  await expect(page.getByText(bobText)).toBeVisible();
  await blockBob(page, bobText);

  // Bob's message and his echo of a thread reply sit together, so they make one row.
  const run = page.getByText("2 blocked messages").locator("..");
  await expect(run).toBeVisible();
  await run.getByRole("button", { name: "Show" }).click();
  await expect(page.getByText(bobText)).toBeVisible();

  const card = await authorCard(page, bobText);
  await expect(card.getByText("Blocked", { exact: true })).toBeVisible();
  await expect(card.getByRole("button", { name: "Message" })).toHaveCount(0);
  await card.getByRole("button", { name: "Unblock" }).click();
  await expect(page.getByText(/blocked message/)).toHaveCount(0);
  await expect(page.getByText(bobText)).toBeVisible();
});

test("a DM with someone blocked offers to unblock, as the settings do", async ({ page }) => {
  await page
    .getByRole("navigation", { name: "Communities" })
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await page
    .getByRole("link", { name: new RegExp(`^${bob}`) })
    .first()
    .click();
  await blockBob(page, "Did you get the photos?");
  await expect(page.getByText("1 blocked message")).toBeVisible();
  const note = page.getByText(`You blocked ${bob}. Unblock them to message each other again.`);
  await expect(note).toBeVisible();
  await expect(page.getByRole("textbox")).toHaveCount(0);

  // A phone shows the conversation alone; the settings are on the list's screen.
  const back = page.getByRole("link", { name: "Back to direct messages" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: "See 1 blocked user" }).click();
  const list = page.getByRole("dialog", { name: "Blocked users" });
  const blocked = list.getByRole("grid", { name: "Blocked users" });
  await expect(blocked.getByText(bob)).toBeVisible();
  await blocked.getByRole("button", { name: "Unblock" }).click();
  await expect(list.getByText("You haven't blocked anyone.")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(settings.getByText("You haven't blocked anyone.")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(note).toHaveCount(0);
});

/** Opens the DM with Bob from the DM list. */
async function openBobsDm(page: Page) {
  await page
    .getByRole("navigation", { name: "Communities" })
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await page
    .getByRole("link", { name: new RegExp(`^${bob}`) })
    .first()
    .click();
  await expect(page.getByText("Did you get the photos?")).toBeVisible();
}

/** Blocks from the card that is open, and checks the DM then offers to unblock. */
async function blockFromOpenCard(page: Page) {
  const card = page.getByRole("dialog", { name: new RegExp(bob) });
  await card.getByRole("button", { name: "Block" }).click();
  await card.getByRole("button", { name: "Block" }).click();
  await expect(card.getByText("Blocked", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByText(`You blocked ${bob}. Unblock them to message each other again.`),
  ).toBeVisible();
}

test("the name in a DM's header opens that person's card", async ({ page }) => {
  await openBobsDm(page);
  await page
    .getByRole("heading", { name: new RegExp(bob) })
    .getByRole("button", { name: `Show profile of ${bob}` })
    .click();
  // Messaging him would lead where the reader already is.
  const card = page.getByRole("dialog", { name: new RegExp(bob) });
  await expect(card.getByRole("button", { name: "Block" })).toBeVisible();
  await expect(card.getByRole("button", { name: "Message" })).toHaveCount(0);
  await blockFromOpenCard(page);
});

test("pressing the open DM in the list opens the other person's card", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows the list or the DM");
  await openBobsDm(page);
  const row = page
    .getByRole("navigation", { name: "Direct messages" })
    .getByRole("button", { name: new RegExp(`^Show profile of ${bob}`) });
  await expect(row).toHaveAttribute("aria-current", "page");
  await row.click();
  await blockFromOpenCard(page);
});

/** How many people the caller of `manyBlocked` has blocked. */
const BLOCKED = 2000;
const trollId = (n: number) => `0190f0a0-0000-7000-8009-${String(n).padStart(12, "0")}`;

/** A caller who has blocked `BLOCKED` people, Troll number 1 to Troll number 2000. */
async function manyBlocked(page: Page, unblocked: string[]) {
  const trolls = Array.from({ length: BLOCKED }, (_, i) =>
    user(trollId(i + 1), `troll${String(i + 1)}`, `Troll number ${String(i + 1)}`),
  );
  await page.route(/\/api\/v1\/users\/(@|%40)me\/blocks(\?.*)?$/, (route) =>
    route.fulfill({
      json: {
        data: trolls.map((troll) => ({ user: troll.id, createdAt: new Date().toISOString() })),
        included: { users: trolls },
      },
    }),
  );
  await page.route(/\/api\/v1\/users\/(@|%40)me\/blocks\/[^/]+$/, (route) => {
    unblocked.push(route.request().url().split("/").at(-1) ?? "");
    return route.fulfill({ status: 204 });
  });
}

test("thousands of blocked users open in a list of their own, found by name or handle", async ({
  page,
}) => {
  const unblocked: string[] = [];
  await manyBlocked(page, unblocked);
  await page.reload();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: "See 2,000 blocked users" }).click();
  const dialog = page.getByRole("dialog", { name: "Blocked users" });
  const grid = dialog.getByRole("grid", { name: "Blocked users" });
  await expect(grid.getByText("Troll number 1", { exact: true })).toBeVisible();
  // Only what shows is drawn, and the list scrolls within the modal.
  expect(await grid.getByRole("row").count()).toBeLessThan(100);
  expect(await grid.evaluate((list) => list.scrollHeight > list.clientHeight)).toBe(true);
  // Every row's button is whole, inside the list's edge.
  expect(
    await grid.evaluate((list) => {
      const edge = list.getBoundingClientRect().right;
      return Array.from(list.querySelectorAll("button")).every(
        (button) => button.getBoundingClientRect().right <= edge,
      );
    }),
  ).toBe(true);
  await grid.evaluate((list) => {
    list.scrollTop = list.scrollHeight;
  });
  await expect(grid.getByText(`Troll number ${String(BLOCKED)}`, { exact: true })).toBeVisible();

  const search = dialog.getByRole("searchbox", { name: "Find a blocked user" });
  await search.fill("TROLL NUMBER 1234");
  await expect(grid.getByRole("row")).toHaveCount(1);
  await search.fill("@troll1999");
  await expect(grid.getByRole("row")).toHaveCount(1);
  await expect(grid.getByText("Troll number 1999", { exact: true })).toBeVisible();
  await search.fill("troll1234");
  await grid.getByRole("button", { name: "Unblock" }).click();
  await expect(dialog.getByText("Nobody you've blocked has that name.")).toBeVisible();
  expect(unblocked).toEqual([trollId(1234)]);
  await search.clear();
  await expect(grid.getByText("Troll number 1", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(settings.getByRole("button", { name: "See 1,999 blocked users" })).toBeVisible();
});
