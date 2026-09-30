import { expect, test, type Page } from "@playwright/test";
import { friendlyDeployment, rekeyedDeployment, signInToWorld } from "./world";

/**
 * Federation in the Administration Dashboard, against the stubbed world in `world.ts`: this
 * deployment lets its users go only where it allows, and knows two deployments, one of which
 * has offered a new key.
 */

const known = (page: Page) => page.getByRole("region", { name: "Known deployments" });
const row = (page: Page, domain: string) =>
  known(page).getByRole("row").filter({ hasText: domain });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
  await page
    .getByRole("navigation", { name: "Communities" })
    .getByRole("link", { name: "Administration" })
    .click();
  await page
    .getByRole("navigation", { name: "Administration sections" })
    .getByRole("link", { name: "Federation" })
    .click();
  await expect(page.getByRole("region", { name: "Federation" })).toBeVisible();
});

test("the section shows this deployment's identity and gates", async ({ page }) => {
  const federation = page.getByRole("region", { name: "Federation" });
  await expect(federation.getByText("aspen.example.com")).toBeVisible();
  await expect(federation.getByText("SHA256:this-deployment")).toBeVisible();
  const gates = federation.getByRole("group", { name: "Gates" });
  await expect(gates.getByRole("row").filter({ hasText: "Users" })).toContainText("Allow list");
  await expect(gates.getByRole("row").filter({ hasText: "Users" })).toContainText("Open");
  await expect(gates.getByRole("row").filter({ hasText: "Bots" })).toContainText("Closed");
});

test("an administrator adds a deployment, which is contacted and pinned at once", async ({
  page,
}) => {
  const add = page.getByRole("form", { name: "Add a deployment" });
  await add.getByRole("textbox", { name: "Domain" }).fill("new.example.org");
  await add.getByRole("textbox", { name: "Note" }).fill("a new friend");
  await add.getByRole("button", { name: "Add" }).click();
  await expect(add.getByRole("status")).toHaveText("Pinned new.example.org's key.");
  await expect(row(page, "new.example.org")).toContainText("SHA256:first-key");
  await expect(row(page, "new.example.org")).toContainText("a new friend");

  await add.getByRole("textbox", { name: "Domain" }).fill(friendlyDeployment);
  await add.getByRole("button", { name: "Add" }).click();
  await expect(add.getByRole("alert")).toHaveText("That deployment is already in the directory.");
});

test("lists decide who may go, and a check says what the deployment presented", async ({
  page,
}) => {
  const friendly = row(page, friendlyDeployment);
  await expect(friendly).toContainText("Users come");
  await expect(friendly).not.toContainText("Users go");
  await friendly.getByRole("button", { name: `Lists for ${friendlyDeployment}` }).click();
  const dialog = page.getByRole("dialog", { name: `Lists for ${friendlyDeployment}` });
  const allow = dialog.getByRole("checkbox", { name: /Users may go there/ });
  await expect(allow).not.toBeChecked();
  await dialog.getByText("Users may go there").click();
  await expect(allow).toBeChecked();
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(friendly).toContainText("Users go");

  await friendly.getByRole("button", { name: `Check ${friendlyDeployment} now` }).click();
  await expect(friendly.getByRole("status")).toHaveText(
    `${friendlyDeployment} presented its pinned key.`,
  );
});

test("a changed key is refused until an administrator reviews and accepts it", async ({ page }) => {
  const rekeyed = row(page, rekeyedDeployment);
  await expect(rekeyed).toContainText("New key offered");
  await rekeyed.getByRole("button", { name: `Check ${rekeyedDeployment} now` }).click();
  await expect(rekeyed.getByRole("status")).toHaveText(
    `${rekeyedDeployment} presented a different key. It is refused until you accept it.`,
  );
  await rekeyed.getByRole("button", { name: `Review ${rekeyedDeployment}'s new key` }).click();
  const review = page.getByRole("alertdialog", { name: `${rekeyedDeployment} offers a new key` });
  await expect(review).toContainText(`SHA256:pinned-${rekeyedDeployment}`);
  await expect(review).toContainText("SHA256:offered-key");
  await review.getByRole("button", { name: "Accept the new key" }).click();
  await expect(review).toBeHidden();
  await expect(rekeyed).toContainText("SHA256:offered-key");
  await expect(rekeyed).not.toContainText("New key offered");
});

test("forgetting a deployment takes it out of the directory", async ({ page }) => {
  await row(page, friendlyDeployment)
    .getByRole("button", { name: `Forget ${friendlyDeployment}` })
    .click();
  const confirm = page.getByRole("alertdialog", { name: `Forget ${friendlyDeployment}?` });
  await confirm.getByRole("button", { name: "Forget" }).click();
  await expect(row(page, friendlyDeployment)).toHaveCount(0);
  await expect(row(page, rekeyedDeployment)).toHaveCount(1);
});
