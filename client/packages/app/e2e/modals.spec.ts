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
  await expectClosesFromCorner(page, async () => {
    // On a narrow screen the poll is behind the box's + button.
    const more = page.getByRole("button", { name: "Add a file or a poll" });
    if (await more.isVisible()) {
      await more.click();
      await page.getByRole("menuitem", { name: "Create a new poll" }).click();
    } else {
      await page.getByRole("button", { name: "Create a new poll" }).click();
    }
  });
});

test("a modal never scrolls sideways, and settings' planes stand in two columns", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a phone shows one column, and its modal fills the screen");
  // As first opened after a load: the planes are measured while the modal is still arriving,
  // scaled up from a little smaller, and a measure taken at that size once set a height the
  // planes overflowed into a third column.
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings", exact: true });
  await expect(dialog).toBeVisible();
  await expect
    .poll(() =>
      dialog.evaluate((element) => {
        const frame = element.closest(".motion-dialog");
        const planes = element.querySelector(".\\@container")?.firstElementChild;
        return {
          overflow: frame === null ? NaN : frame.scrollWidth - frame.clientWidth,
          columns:
            planes === null || planes === undefined
              ? 0
              : new Set(
                  Array.from(planes.children, (c) => Math.round(c.getBoundingClientRect().left)),
                ).size,
        };
      }),
    )
    .toEqual({ overflow: 0, columns: 2 });
});
