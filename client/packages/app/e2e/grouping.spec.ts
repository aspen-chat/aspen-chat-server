import { expect, test, type Page } from "@playwright/test";
import { bob, general, longPress, me, signInToWorld } from "./world";
import { message } from "./world/fixtures";

/**
 * One author's messages drawn as a group, against the stubbed world in `world.ts` with a
 * history of its own: Bob's four within minutes, then the caller's three, then Bob again forty
 * minutes on, which begins a group of its own.
 */

/** A link long enough to wrap over several lines, whose last line leaves room after it. */
const longLink = "https://example.com/" + "segment/".repeat(30) + "?query=" + "abc".repeat(30);

const history = [
  message(311, bob, `A very long URL: ${longLink}`, 0),
  message(310, bob, "Back again after forty minutes.", 1),
  message(309, me, "Third of mine.", 41),
  message(308, me, "Second of mine.", 42),
  message(307, me, "My first.", 43),
  message(306, bob, "Four.", 44),
  message(305, bob, "Three.", 45),
  message(304, bob, "Two.", 46),
  message(303, bob, "One.", 47),
];

async function serveHistory(page: Page) {
  await page.route(`**/api/v1/channels/${general}/messages*`, (route) =>
    route.fulfill({ json: { data: history, included: {} } }),
  );
}

const row = (page: Page, text: string) =>
  page.locator("article[data-message-id]").filter({ hasText: text });
const named = (page: Page, text: string) =>
  row(page, text).getByRole("button", { name: /^Show profile of / });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page, serveHistory);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(row(page, "Back again")).toBeVisible();
});

test("a run of one author's messages shows the author once, and a long gap starts again", async ({
  page,
}) => {
  for (const first of ["One.", "My first.", "Back again"]) {
    await expect(named(page, first)).not.toHaveCount(0);
  }
  for (const grouped of ["Two.", "Three.", "Four.", "Second of mine.", "Third of mine."]) {
    await expect(named(page, grouped)).toHaveCount(0);
    // Its author is still read out with it.
    await expect(row(page, grouped).locator(".sr-only")).toContainText(
      grouped.endsWith("mine.") ? "Kate" : "Bob",
    );
  }
});

test("a grouped message shows its time after its text while the pointer is over it", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a phone's actions say when it was sent instead");
  const time = row(page, "Three.").locator("time[aria-hidden]");
  await expect(time).toBeHidden();
  await row(page, "Three.").hover();
  await expect(time).toBeVisible();
});

test("a grouped message's time ends the last line of its text where the line has room", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "a phone's actions say when it was sent instead");
  const long = row(page, "A very long URL");
  const link = long.getByRole("link", { name: /example\.com/ });
  const lines = await link.evaluate((a) =>
    Array.from(a.getClientRects(), (r) => ({ top: r.top, bottom: r.bottom, right: r.right })),
  );
  expect(lines.length).toBeGreaterThan(1);
  const height = (await long.boundingBox())?.height;
  await long.hover();
  const time = long.locator("time[aria-hidden]");
  await expect(time).toBeVisible();
  const box = await time.boundingBox();
  const last = lines.at(-1);
  if (box === null || last === undefined) {
    throw new Error("the time or the link's last line has no box");
  }
  // Beside the link's last line, not under it, and the message no taller for it.
  expect(box.y + box.height / 2).toBeGreaterThan(last.top);
  expect(box.y + box.height / 2).toBeLessThan(last.bottom);
  expect(box.x).toBeGreaterThanOrEqual(last.right);
  expect((await long.boundingBox())?.height).toBe(height);
});

test("a long press says when a grouped message was sent", async ({ page, isMobile }) => {
  test.skip(!isMobile, "a long press is a touch screen's");
  await longPress(page, row(page, "Three.").locator(".message-body"));
  await expect(
    page.getByRole("dialog", { name: "Message actions" }).getByText(/^Sent at /),
  ).toBeVisible();
});
