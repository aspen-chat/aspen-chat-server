import { expect, test } from "@playwright/test";
import { gifPageLink, missingPictureLink, signInToWorld } from "./world";

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
  const article = page.locator("article").filter({ hasText: "missing-picture.png" });
  await expect(article.getByRole("link", { name: missingPictureLink })).toBeVisible();
  await expect(article.getByRole("img")).toHaveCount(0);
});
