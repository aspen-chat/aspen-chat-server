import { expect, test } from "@playwright/test";
import { gifPageLink, missingPictureLink, missingPictureMessageId, signInToWorld } from "./world";
import { PIXEL_PNG } from "./world/fixtures";

/** Every request made of the picture link's own host, which must stay empty. */
let askedOfLinkHost: string[] = [];

test.beforeEach(async ({ page }) => {
  askedOfLinkHost = [];
  await page.route(missingPictureLink, (route) => {
    askedOfLinkHost.push(route.request().url());
    return route.fulfill({ status: 404 });
  });
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
});

test("a share page named like a gif shows the picture the server found there", async ({ page }) => {
  const article = page.locator("article").filter({ has: page.locator(`[alt*="${gifPageLink}"]`) });
  await expect(article).toHaveCount(1);
  const picture = article.getByRole("img");
  await expect(picture).toHaveAttribute("src", PIXEL_PNG);
  // The page's own address is never asked for a picture.
  await expect(article.locator(`img[src="${gifPageLink}"]`)).toHaveCount(0);
});

test("a picture link the server has no picture for shows as a link, never loaded", async ({
  page,
}) => {
  const article = page.locator(`article[data-message-id="${missingPictureMessageId}"]`);
  await article.scrollIntoViewIfNeeded();
  await expect(article.getByRole("link", { name: missingPictureLink })).toBeVisible();
  await expect(article.getByRole("img")).toHaveCount(0);
  // Loading it would tell the link's host who reads the message.
  expect(askedOfLinkHost).toEqual([]);
});
