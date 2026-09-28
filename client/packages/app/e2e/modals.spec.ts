import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Every modal closes from an X in its top right corner, and has no other button that only
 * closes it. Checked on several modals of the stubbed world in `world.ts`.
 */

async function expectClosesFromCorner(page: Page, open: () => Promise<void>) {
  await open();
  const dialog = page.getByRole("dialog").last();
  await expect(dialog).toBeVisible();
  const close = dialog.getByRole("button", { name: "Close", exact: true });
  await expect(close).toHaveCount(1);
  for (const name of ["Cancel", "Done", "OK"]) {
    await expect(dialog.getByRole("button", { name, exact: true })).toHaveCount(0);
  }
  const frame = await dialog.boundingBox();
  const corner = await close.boundingBox();
  if (frame === null || corner === null) {
    throw new Error("the dialog or its X has no box");
  }
  // In the top right: its right edge near the dialog's, its top in the top band.
  expect(frame.x + frame.width - (corner.x + corner.width)).toBeLessThan(8);
  expect(corner.y - frame.y).toBeLessThan(16);
  await close.click();
  await expect(dialog).toBeHidden();
}

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("settings, invites, and new polls close from their X", async ({ page }) => {
  await expectClosesFromCorner(page, () =>
    page.getByRole("button", { name: "Settings", exact: true }).click(),
  );
  await expectClosesFromCorner(page, () =>
    page.getByRole("button", { name: "Community settings" }).click(),
  );
  await expectClosesFromCorner(page, () =>
    page.getByRole("button", { name: "Invite people" }).click(),
  );
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expectClosesFromCorner(page, () =>
    page.getByRole("button", { name: "Create a new poll" }).click(),
  );
});
