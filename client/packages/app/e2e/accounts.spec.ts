import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";
import { user } from "./world/fixtures";

const priya = "0190f0a0-0000-7000-8000-0000000000a1";
const PRIYA_TOKEN = "priya-session";

/** Answers as Priya whenever her token is presented, and signs her in by her username. */
async function stubSecondPerson(page: Page): Promise<void> {
  await page.route(/\/api\/v1\/auth\/login$/, async (route) => {
    const body = route.request().postDataJSON() as { username?: string };
    if (body.username !== "priya") {
      await route.fallback();
      return;
    }
    await route.fulfill({
      json: {
        status: "signedIn",
        userId: priya,
        refreshToken: "priya-refresh",
        sessionToken: PRIYA_TOKEN,
        sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
      },
    });
  });
  await page.route(/\/api\/v1\/users\/(@|%40)me(\?.*)?$/, async (route) => {
    const asPriya = route.request().headers().authorization === `Bearer ${PRIYA_TOKEN}`;
    if (route.request().method() === "GET" && asPriya) {
      await route.fulfill({ json: user(priya, "priya", "Priya") });
    } else {
      await route.fallback();
    }
  });
}

async function openSettings(page: Page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("dialog", { name: "Settings", exact: true });
}

/** Turns several accounts on and signs Priya in beside Kate. */
async function addPriya(page: Page, password = "hunter22") {
  const settings = await openSettings(page);
  await settings.getByRole("checkbox", { name: /Use several accounts/ }).click({ force: true });
  await settings.getByRole("button", { name: "Add account", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Add an account" });
  await dialog.getByLabel("Username").fill("priya");
  await dialog.getByLabel("Password").fill(password);
  await dialog.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(dialog).toBeHidden();
}

/** Whose app is shown, as the foot of the sidebar names them. */
const shownAs = (page: Page, name: string) =>
  expect(
    page.getByRole("button", { name: /^Change your status/ }).filter({ hasText: name }),
  ).toBeVisible();

const sessionKeys = (page: Page) =>
  page.evaluate(() => Object.keys(localStorage).filter((key) => key.startsWith("aspen.session")));

test.describe("several accounts", () => {
  test("is off until Settings turns it on, with nothing new on screen", async ({ page }) => {
    await signInToWorld(page, stubSecondPerson);
    await expect(page.getByRole("button", { name: "Switch accounts" })).toHaveCount(0);
    const settings = await openSettings(page);
    await expect(settings.getByRole("button", { name: "Add account" })).toHaveCount(0);
    await expect(
      settings.getByRole("checkbox", { name: /Use several accounts/ }),
    ).not.toBeChecked();
  });

  test("adds a second account, shown at once, each with its own session", async ({ page }) => {
    await signInToWorld(page, stubSecondPerson);
    await addPriya(page);
    await shownAs(page, "Priya");
    // The world's community is Kate's: Priya is no member, so has no message box there.
    await page.getByRole("button", { name: "Switch accounts" }).click();
    await page.getByRole("menuitem", { name: /Kate/ }).click();
    await shownAs(page, "Kate");
    await page
      .getByRole("row", { name: /^general/ })
      .getByText("general", { exact: true })
      .click();
    await expect(page.getByText("Posting as Kate")).toBeVisible();
    const keys = await sessionKeys(page);
    expect(keys).toHaveLength(2);
    expect(keys).toContain("aspen.session");
    const accounts = await page.evaluate(() => localStorage.getItem("aspen.accounts"));
    expect(accounts).not.toContain("refresh");
  });

  test("switches between accounts, and each window stays on its own", async ({ page }) => {
    await signInToWorld(page, stubSecondPerson);
    await addPriya(page);
    await shownAs(page, "Priya");
    await page.getByRole("button", { name: "Switch accounts" }).click();
    await page.getByRole("menuitem", { name: /Kate/ }).click();
    await shownAs(page, "Kate");
    await page.reload();
    await shownAs(page, "Kate");
    await page.getByRole("button", { name: "Switch accounts" }).click();
    await page.getByRole("menuitem", { name: /Priya/ }).click();
    await shownAs(page, "Priya");
    await page.reload();
    await shownAs(page, "Priya");
  });

  test("signs one account out and keeps the other signed in", async ({ page }) => {
    await signInToWorld(page, stubSecondPerson);
    await addPriya(page);
    await shownAs(page, "Priya");
    const settings = await openSettings(page);
    await settings.getByRole("button", { name: "Sign out of Priya" }).click();
    await settings.getByRole("button", { name: "Sign out of Priya" }).click();
    await shownAs(page, "Kate");
    await expect.poll(() => sessionKeys(page)).toEqual(["aspen.session"]);
    await expect(page.getByLabel("Password")).toHaveCount(0);
  });

  test("refuses the same account twice and keeps one session of it", async ({ page }) => {
    await signInToWorld(page, stubSecondPerson);
    await addPriya(page);
    await shownAs(page, "Priya");
    const settings = await openSettings(page);
    await settings.getByRole("button", { name: "Add account", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Add an account" });
    await dialog.getByLabel("Username").fill("priya");
    await dialog.getByLabel("Password").fill("hunter22");
    await dialog.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByText("That account is already signed in here.").first()).toBeAttached();
    await expect.poll(() => sessionKeys(page)).toHaveLength(2);
  });

  test("turning it off asks, then signs the others out", async ({ page }) => {
    await signInToWorld(page, stubSecondPerson);
    await addPriya(page);
    await shownAs(page, "Priya");
    const settings = await openSettings(page);
    await settings.getByRole("checkbox", { name: /Use several accounts/ }).click({ force: true });
    await expect(settings.getByText(/signs out of 1 other account/)).toBeVisible();
    expect(await sessionKeys(page)).toHaveLength(2);
    await settings.getByRole("button", { name: "Sign out and turn off" }).click();
    await expect(page.getByRole("button", { name: "Switch accounts" })).toHaveCount(0);
    await expect.poll(() => sessionKeys(page)).toHaveLength(1);
  });
});
