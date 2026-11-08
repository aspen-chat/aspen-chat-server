import { expect, test, type Locator, type Page } from "@playwright/test";
import { settleAnimations, signInToWorld } from "./world";

/**
 * Every modal closes from an X in its top right corner, and has no other button that only
 * closes it. Checked on several modals of the stubbed world in `world.ts`.
 */

/** The frame a dialog is drawn in: the scroll container of `modalFrameClass`. */
function frameOf(dialog: Locator) {
  return dialog.locator("xpath=ancestor::*[contains(concat(' ', @class, ' '), ' motion-dialog ')]");
}

/** Wider than the frame's padding on any screen, and the border beside it. */
const PADDING_BAND = 24;

async function expectClosesFromCorner(page: Page, open: () => Promise<void>) {
  await open();
  const dialog = page.getByRole("dialog").last();
  await expect(dialog).toBeVisible();
  const close = dialog.getByRole("button", { name: "Close", exact: true });
  await expect(close).toHaveCount(1);
  for (const name of ["Cancel", "Done", "OK"]) {
    await expect(dialog.getByRole("button", { name, exact: true })).toHaveCount(0);
  }
  const frame = await frameOf(dialog).boundingBox();
  const corner = await close.boundingBox();
  if (frame === null || corner === null) {
    throw new Error("the dialog or its X has no box");
  }
  // In the top right: within the frame's padding band at its top and at its end.
  expect(frame.x + frame.width - (corner.x + corner.width)).toBeLessThan(PADDING_BAND);
  expect(corner.y - frame.y).toBeLessThan(PADDING_BAND);
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

test("a stepped dialog keeps its X in the corner on every step", async ({ page }) => {
  const addNew = () => page.getByRole("button", { name: "Add new…" }).click();
  await expectClosesFromCorner(page, addNew);
  await expectClosesFromCorner(page, async () => {
    await addNew();
    await page.getByRole("button", { name: /^Text channel/ }).click();
    await expect(page.getByRole("button", { name: "Back", exact: true })).toBeVisible();
  });
});

test("a modal never scrolls sideways, and settings' planes stand in three columns", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a phone shows one column, and its modal fills the screen");
  // As first opened after a load: the planes are measured while the modal is still arriving,
  // scaled up from a little smaller, and a measure taken at that size once set a height the
  // planes overflowed into a column too many. A desktop window is wide enough for three.
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
    .toEqual({ overflow: 0, columns: 3 });
});

test("a modal's title and X stay at the top while it scrolls", async ({ page }) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings", exact: true });
  await expect(dialog).toBeVisible();
  await settleAnimations(page);
  const frame = frameOf(dialog);
  const scrolled = await frame.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
    return element.scrollTop;
  });
  expect(scrolled, "settings holds more than the screen shows").toBeGreaterThan(0);
  const frameBox = await frame.boundingBox();
  const title = dialog.getByRole("heading", { name: "Settings", exact: true });
  const titleBox = await title.boundingBox();
  const close = dialog.getByRole("button", { name: "Close", exact: true });
  const closeBox = await close.boundingBox();
  if (frameBox === null || titleBox === null || closeBox === null) {
    throw new Error("the frame, its title, or its X has no box");
  }
  // Still in the frame's padding band at the top, with nothing showing through beside them.
  expect(titleBox.y - frameBox.y).toBeLessThan(PADDING_BAND);
  expect(closeBox.y - frameBox.y).toBeLessThan(PADDING_BAND);
  const covered = await page.evaluate(
    ({ x, y }) => document.elementFromPoint(x, y)?.closest(".pinned-title") !== null,
    { x: frameBox.x + frameBox.width / 2, y: frameBox.y + 2 },
  );
  expect(covered, "the title row's ground covers what scrolls under it").toBe(true);
  // A control above the view that takes focus is scrolled to below the row, not under it.
  const security = dialog.getByRole("button", { name: "Sign-in and security" });
  await security.focus();
  const securityBox = await security.boundingBox();
  const rowBox = await dialog.locator(".pinned-title").boundingBox();
  if (securityBox === null || rowBox === null) {
    throw new Error("the focused control or the title row has no box");
  }
  expect(securityBox.y).toBeGreaterThanOrEqual(rowBox.y + rowBox.height);
  await close.click();
  await expect(dialog).toBeHidden();
});

/** The frame fills the screen edge to edge, with no corner rounded, or keeps a margin all round. */
async function expectFills(page: Page, dialog: Locator, fills: boolean) {
  await expect(dialog).toBeVisible();
  await settleAnimations(page);
  const frame = frameOf(dialog);
  const shape = await frame.evaluate((element) => {
    const box = element.getBoundingClientRect();
    return {
      left: Math.round(box.left),
      top: Math.round(box.top),
      right: Math.round(window.innerWidth - box.right),
      bottom: Math.round(window.innerHeight - box.bottom),
      radius: getComputedStyle(element).borderTopLeftRadius,
    };
  });
  if (fills) {
    expect(shape).toEqual({ left: 0, top: 0, right: 0, bottom: 0, radius: "0px" });
  } else {
    expect(shape.left).toBeGreaterThan(0);
    expect(shape.right).toBeGreaterThan(0);
    expect(shape.radius).not.toBe("0px");
  }
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toBeHidden();
}

test("on a touch-only screen the large modals fill it and the small ones keep their margin", async ({
  page,
  isMobile,
}) => {
  test.skip(!isMobile, "a phone's screen is touch-only; a desktop's is not");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expectFills(page, page.getByRole("dialog", { name: "Settings", exact: true }), true);
  await page.getByRole("button", { name: "Community settings" }).click();
  await expectFills(page, page.getByRole("dialog").last(), true);
  await page.getByRole("button", { name: "Invite people" }).click();
  await expectFills(page, page.getByRole("dialog").last(), false);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await page.getByRole("button", { name: "Search messages" }).click();
  await expectFills(page, page.getByRole("dialog", { name: "Search messages" }), true);
  await page.getByRole("button", { name: "Add a file or a poll" }).click();
  await page.getByRole("menuitem", { name: "Create a new poll" }).click();
  await expectFills(page, page.getByRole("dialog").last(), true);
});
