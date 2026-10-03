import { expect, test } from "@playwright/test";
import { gifPageLink, missingPictureLink, missingPictureMessageId, signInToWorld } from "./world";

test.beforeEach(async ({ page }) => {
  await page.route(missingPictureLink, (route) => route.fulfill({ status: 404 }));
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
  await expect(picture).toHaveAttribute("src", /^data:image\/png/);
  // The page's own address is never asked for a picture.
  await expect(article.locator(`img[src="${gifPageLink}"]`)).toHaveCount(0);
});

test("a picture link that answers nothing shows as a link, not a broken picture", async ({
  page,
}) => {
  // Found by its id, since the link's text shows only once the picture has failed, and the
  // picture loads lazily, asked for (and failing) only once it comes into view, which on a
  // phone's short screen takes a scroll.
  const article = page.locator(`article[data-message-id="${missingPictureMessageId}"]`);
  await article.scrollIntoViewIfNeeded();
  await expect(article.getByRole("link", { name: missingPictureLink })).toBeVisible();
  await expect(article.getByRole("img")).toHaveCount(0);
});
