import { expect, test, type Page } from "@playwright/test";
import { helper, PIXEL_PNG, signInToWorld } from "./world";

/**
 * A person's card opens their picture four times as large, against the stubbed world in
 * `world.ts`, where Helper is given an icon by an update once signed in.
 */

const ICON = "019a0000-0000-7000-8000-0000000000f1";

async function openHelpersCard(page: Page, isMobile: boolean) {
  const publish = await signInToWorld(page, async (page) => {
    await page.route(`**/api/v1/icons/${ICON}`, (route) =>
      route.fulfill({ json: { id: ICON, mimeType: "image/png", downloadUrl: PIXEL_PNG } }),
    );
  });
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  publish({ serverEvent: "user", type: "update", id: helper, icon: ICON });
  if (isMobile) {
    await page.getByRole("button", { name: "Show members" }).first().click();
  }
  await page
    .getByRole("complementary", { name: "Members" })
    .or(page.getByRole("dialog", { name: "Members" }))
    .getByRole("button", { name: /^Show profile of Helper/ })
    .click();
  return page.getByRole("dialog", { name: "Profile of Helper" });
}

async function boxOf(locator: ReturnType<Page["locator"]>) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  return box ?? { x: 0, y: 0, width: 0, height: 0 };
}

test("the card's picture opens four times as large and goes at a click elsewhere", async ({
  page,
  isMobile,
}) => {
  const card = await openHelpersCard(page, isMobile);
  const picture = card.getByRole("button", { name: "Show Helper's picture larger" });
  const zoom = page.getByRole("dialog", { name: "Helper's picture" });
  if (isMobile) {
    await picture.tap();
  } else {
    await picture.hover();
  }
  await expect(zoom).toBeVisible();
  const small = await boxOf(picture.locator("img"));
  const large = await boxOf(zoom.locator("img"));
  expect(small.width).toBeGreaterThan(0);
  expect(large.width).toBeCloseTo(small.width * 4, 0);
  expect(large.height).toBeCloseTo(small.height * 4, 0);

  // A click on the card puts the picture away and leaves the card. It lands on the overlay
  // under the picture, as a person's would, which Playwright's own click refuses to do.
  const handle = await boxOf(card.getByText("@helper"));
  await page.mouse.click(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await expect(zoom).toHaveCount(0);
  await expect(card).toBeVisible();

  // Escape puts the picture away before the card.
  await picture.click();
  await expect(zoom).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(zoom).toHaveCount(0);
  await expect(card).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(card).toHaveCount(0);
});

test("the larger picture goes with the card", async ({ page, isMobile }) => {
  const card = await openHelpersCard(page, isMobile);
  await card.getByRole("button", { name: "Show Helper's picture larger" }).click();
  const zoom = page.getByRole("dialog", { name: "Helper's picture" });
  await expect(zoom).toBeVisible();
  // Leaving the channel takes the member list, and with it the card and what it opened.
  await page.evaluate(() => {
    window.history.pushState(null, "", "/");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  await expect(card).toHaveCount(0);
  await expect(zoom).toHaveCount(0);
});
