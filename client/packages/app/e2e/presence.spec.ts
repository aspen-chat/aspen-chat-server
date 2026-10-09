import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Choosing a status from the user bar, against the stubbed world in `world.ts`, where #roadmap
 * and the DM with Bob hold messages the caller has not read, #roadmap's tagging them twice.
 */

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });
const statusMenu = (page: Page) => page.getByRole("menu", { name: "Your status" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("do not disturb hides unread marks until the user is online again", async ({ page }) => {
  await expect(page.getByText("roadmap, unread")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: /^Family, unread/ })).toBeVisible();
  const put = page.waitForRequest(
    (request) => request.method() === "PUT" && request.url().endsWith("/presence-override"),
  );
  await page.getByRole("button", { name: "Change your status: Online" }).click();
  await statusMenu(page)
    .getByRole("menuitem", { name: /^Do not disturb/ })
    .click();
  await page
    .getByRole("menu", { name: "Do not disturb" })
    .getByRole("menuitem", { name: "For 1 hour" })
    .click();
  expect((await put).postDataJSON()).toEqual({
    presenceOverride: "doNotDisturb",
    durationSeconds: 3600,
  });
  // Their picture's mark says so, and nothing is marked unread or counted.
  await expect(
    page.getByRole("button", { name: "Change your status: Do not disturb" }),
  ).toBeVisible();
  await expect(page.getByText("roadmap, unread")).toHaveCount(0);
  await expect(rail(page).getByRole("row", { name: "Family", exact: true })).toBeVisible();
  await expect(
    rail(page).getByRole("link", { name: "Direct messages", exact: true }),
  ).toBeVisible();

  // The menu says until when, and Online ends it.
  await page.getByRole("button", { name: "Change your status: Do not disturb" }).click();
  await expect(statusMenu(page).getByRole("menuitem", { name: /Until / })).toBeVisible();
  const deleted = page.waitForRequest(
    (request) => request.method() === "DELETE" && request.url().endsWith("/presence-override"),
  );
  await statusMenu(page)
    .getByRole("menuitem", { name: /^Online/ })
    .click();
  await deleted;
  await expect(page.getByRole("button", { name: "Change your status: Online" })).toBeVisible();
  await expect(page.getByText("roadmap, unread")).toBeAttached();
});

test("invisible and away last until changed when asked to", async ({ page }) => {
  await page.getByRole("button", { name: "Change your status: Online" }).click();
  await statusMenu(page)
    .getByRole("menuitem", { name: /^Invisible/ })
    .click();
  const put = page.waitForRequest(
    (request) => request.method() === "PUT" && request.url().endsWith("/presence-override"),
  );
  await page
    .getByRole("menu", { name: "Invisible" })
    .getByRole("menuitem", { name: "Until I change it" })
    .click();
  expect((await put).postDataJSON()).toEqual({
    presenceOverride: "invisible",
    durationSeconds: null,
  });
  await expect(page.getByRole("button", { name: "Change your status: Invisible" })).toBeVisible();

  await page.getByRole("button", { name: "Change your status: Invisible" }).click();
  await expect(
    statusMenu(page).getByRole("menuitem", { name: /^Invisible.*Until you change it/ }),
  ).toBeVisible();
  await statusMenu(page).getByRole("menuitem", { name: /^Away/ }).click();
  await page
    .getByRole("menu", { name: "Away" })
    .getByRole("menuitem", { name: "For 15 minutes" })
    .click();
  await expect(page.getByRole("button", { name: "Change your status: Away" })).toBeVisible();
});
