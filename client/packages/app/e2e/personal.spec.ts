import { expect, test, type Page } from "@playwright/test";
import { longPress, reactedText, signInToWorld, systemTextScale } from "./world";

/**
 * Saved messages and the activity feed, against the stubbed world in `world.ts`: saving Bob's
 * "Sounds good" from its actions marks it and lists it under Saved messages, a tab of the page
 * the rail's Activity button opens, where it can be removed; the feed opens on what is unread, a
 * channel to an entry that can be marked read, alone or all at once; its other tabs hold Bob's
 * DM, each message a way to go to it, and, once the caller follows the thread, one entry for
 * it: its latest replies at the message text size, and its message box.
 */

const channels = (page: Page) => page.getByRole("grid", { name: "Channels" }).first();
const soundsGood = (page: Page) => page.locator("article").filter({ hasText: reactedText }).last();

/** Opens "Sounds good"'s actions, hovered where there is a pointer and pressed long on a phone, and picks `action`. */
async function messageAction(page: Page, isMobile: boolean, action: string) {
  const message = soundsGood(page);
  if (isMobile) {
    await longPress(page, message.locator(".message-body"));
    await page
      .getByRole("dialog", { name: "Message actions" })
      .getByRole("button", { name: action })
      .tap();
  } else {
    await message.hover();
    await message.getByRole("button", { name: action }).click();
  }
}

/** Goes to one of the rail's pages, from the channel list on a phone. */
async function openFromRail(page: Page, isMobile: boolean, name: string) {
  if (isMobile) {
    await page.goBack();
  }
  await page
    .getByRole("navigation", { name: "Communities" })
    .getByRole("link", { name, exact: true })
    .click();
}

test("a saved message is marked, listed, and removed", async ({ page, isMobile }) => {
  await signInToWorld(page);
  await channels(page).getByText("general", { exact: true }).click();
  await messageAction(page, isMobile, "Save message");
  await expect(soundsGood(page).getByText("Saved", { exact: true })).toBeAttached();
  // One button on the rail stands for both pages; Saved messages is a tab of what it opens.
  const rail = page.getByRole("navigation", { name: "Communities" });
  if (isMobile) {
    await page.goBack();
  }
  await expect(rail.getByRole("link", { name: "Saved messages" })).toHaveCount(0);
  await rail.getByRole("link", { name: "Activity", exact: true }).click();
  await page
    .getByRole("navigation", { name: "your activity and saved messages" })
    .getByRole("link", { name: "Saved messages" })
    .click();
  await expect(page).toHaveURL(/\/saved$/);

  const list = page.getByRole("list", { name: "Saved messages" });
  await expect(list.getByText(reactedText)).toBeVisible();
  await expect(list.getByText("#general in Family")).toBeVisible();
  await list.getByRole("button", { name: "Remove from saved" }).click();
  await expect(page.getByText(/^Nothing saved yet/)).toBeVisible();
});

const tab = (page: Page, name: string) => page.getByRole("tab", { name, exact: true });

test("the feed opens on what is unread, a channel to an entry, and marks it read", async ({
  page,
}) => {
  await signInToWorld(page);
  // Straight to the feed: opening a channel on the way would read it.
  await page.goto("/activity");
  await expect(tab(page, "Unread")).toHaveAttribute("aria-selected", "true");
  const unread = page.getByRole("list", { name: "Unread", exact: true });
  const entries = unread.locator("> li");
  // #roadmap, the DM, and #general, by their newest unread message.
  await expect(entries).toHaveCount(3);
  await expect(entries.nth(0)).toContainText("#roadmap in Family");
  await expect(entries.nth(1)).toContainText("Direct message");
  await expect(entries.nth(1).getByText("Did you get the photos?")).toBeVisible();
  await expect(entries.nth(1)).toContainText("1 unread");
  await expect(entries.nth(2)).toContainText("#general in Family");

  // Marking one read takes its entry away; the others stay.
  await entries.nth(2).getByRole("button", { name: "Mark read" }).click();
  await expect(entries).toHaveCount(2);

  // Marking all read reports every one that is left in one request.
  const reported = page.waitForRequest(
    (request) =>
      request.method() === "PUT" &&
      new URL(request.url()).pathname.endsWith("/users/@me/read-states"),
  );
  await page.getByRole("button", { name: "Mark all read" }).click();
  const body = (await reported).postDataJSON() as { positions: unknown[] };
  expect(body.positions).toHaveLength(2);
  await expect(page.getByText("You're all caught up.")).toBeVisible();
});

test("the feed's tabs hold DMs, mentions, followed threads, and everything, as filtered", async ({
  page,
  isMobile,
}) => {
  await signInToWorld(page);
  await channels(page).getByText("general", { exact: true }).click();
  await openFromRail(page, isMobile, "Activity");
  const feed = page.getByRole("list", { name: "Activity" });
  const viewAsked = (view: string) =>
    page.waitForRequest((request) => {
      const url = new URL(request.url());
      return (
        url.pathname.endsWith("/users/@me/activity") &&
        url.searchParams.get("filter[view]") === view
      );
    });

  let asked = viewAsked("dms");
  await tab(page, "DMs").click();
  await asked;
  await expect(feed.getByText("Did you get the photos?")).toBeVisible();
  // The tab is remembered.
  await page.goto("/activity");
  await expect(tab(page, "DMs")).toHaveAttribute("aria-selected", "true");

  asked = viewAsked("mentions");
  await tab(page, "Mentions").click();
  await asked;
  await expect(feed.getByText("Did you get the photos?")).toBeVisible();

  await tab(page, "Threads").click();
  await expect(page.getByText(/^Nothing here yet\. Replies in threads you follow/)).toBeVisible();

  // Following the thread brings it as one entry: its latest replies, the caller's own among
  // them, over a message box to reply in place.
  await page.goto("/");
  await channels(page).getByText("general", { exact: true }).click();
  await page
    .getByRole("link", { name: /^View thread/ })
    .first()
    .click();
  await page.getByRole("button", { name: "Follow thread" }).click();
  await expect(page.getByRole("button", { name: "Unfollow thread" })).toBeVisible();
  await page.goto("/activity");
  await expect(tab(page, "Threads")).toHaveAttribute("aria-selected", "true");
  await expect(feed.getByText("Crisps and a fruit platter from me.")).toBeVisible();
  const replies = feed.getByRole("list", { name: "Latest replies" });
  await expect(replies).toHaveCount(1);
  await expect(replies.getByRole("listitem")).toHaveCount(2);
  await expect(replies.getByText("Perfect, thank you!")).toBeVisible();
  await expect(feed.getByRole("textbox")).toBeVisible();
  const scale = await systemTextScale(page);
  await expect
    .poll(async () =>
      Number.parseFloat(
        await replies
          .locator(".message-body")
          .first()
          .evaluate((element) => getComputedStyle(element).fontSize),
      ),
    )
    .toBeCloseTo(16 * scale, 2);

  // All holds what is said in channels too, whatever would notify.
  await tab(page, "All").click();
  await expect(feed.getByText(reactedText)).toBeVisible();
  await expect(feed.getByText("Did you get the photos?")).toBeVisible();

  // Leaving DMs out asks for none, and is remembered.
  const show = page.getByRole("button", { name: "Show", exact: true });
  if ((await show.getAttribute("aria-expanded")) !== "true") {
    await show.click();
  }
  const filtered = page.waitForRequest((request) =>
    new URL(request.url()).pathname.endsWith("/users/@me/activity"),
  );
  const filters = page.getByRole("navigation", { name: "your activity and saved messages" });
  await filters.getByText("Direct messages", { exact: true }).click();
  await expect(filters.getByRole("checkbox", { name: "Direct messages" })).not.toBeChecked();
  expect(new URL((await filtered).url()).searchParams.get("filter[dms]")).toBe("false");
  await expect(feed.getByText("Did you get the photos?")).toHaveCount(0);
});
