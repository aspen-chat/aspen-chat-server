import { expect, test, type Locator, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

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
  const blocked = settings.getByRole("list", { name: "Blocked users" });
  await expect(blocked.getByText(bob)).toBeVisible();
  await blocked.getByRole("button", { name: "Unblock" }).click();
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
