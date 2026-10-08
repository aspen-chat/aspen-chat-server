import { expect, test, type Page } from "@playwright/test";
import { longPress, reactedText, signInToWorld } from "./world";

/**
 * Saved messages and the activity feed, against the stubbed world in `world.ts`: saving Bob's
 * "Sounds good" from its actions marks it and lists it under Saved messages, where it can be
 * removed; the feed holds Bob's DM, and his replies in the thread once the caller follows it,
 * each a way to go to it, and a reply in a thread a way to reply there.
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

/** Goes to one of the user bar's pages from the channel list. */
async function openFromUserBar(page: Page, isMobile: boolean, name: string) {
  if (isMobile) {
    await page.goBack();
  }
  await page.getByRole("link", { name, exact: true }).first().click();
}

test("a saved message is marked, listed, and removed", async ({ page, isMobile }) => {
  await signInToWorld(page);
  await channels(page).getByText("general", { exact: true }).click();
  await messageAction(page, isMobile, "Save message");
  await expect(soundsGood(page).getByText("Saved", { exact: true })).toBeAttached();
  await openFromUserBar(page, isMobile, "Saved messages");
  await expect(page).toHaveURL(/\/saved$/);
  const list = page.getByRole("list", { name: "Saved messages" });
  await expect(list.getByText(reactedText)).toBeVisible();
  await expect(list.getByText("#general in Family")).toBeVisible();
  await list.getByRole("button", { name: "Remove from saved" }).click();
  await expect(page.getByText(/^Nothing saved yet/)).toBeVisible();
});

test("the feed holds what notifies, follows threads, and leaves out what is filtered", async ({
  page,
  isMobile,
}) => {
  await signInToWorld(page);
  await channels(page).getByText("general", { exact: true }).click();
  await openFromUserBar(page, isMobile, "Activity");
  await expect(page).toHaveURL(/\/activity$/);
  const feed = page.getByRole("list", { name: "Activity" });
  await expect(feed.getByText("Did you get the photos?")).toBeVisible();
  await expect(feed.getByText("Crisps and a fruit platter from me.")).toHaveCount(0);

  // Following the thread brings its replies, with a way to reply in place.
  await page.goto("/");
  await channels(page).getByText("general", { exact: true }).click();
  await page.getByRole("link", { name: /^View thread/ }).first().click();
  await page.getByRole("button", { name: "Follow thread" }).click();
  await expect(page.getByRole("button", { name: "Unfollow thread" })).toBeVisible();
  await page.goto("/activity");
  await expect(feed.getByText("Crisps and a fruit platter from me.")).toBeVisible();
  await feed.getByRole("button", { name: "Reply" }).first().click();
  await expect(feed.getByRole("textbox")).toBeVisible();

  // Leaving DMs out asks for none, and is remembered.
  const show = page.getByRole("button", { name: "Show", exact: true });
  if ((await show.getAttribute("aria-expanded")) !== "true") {
    await show.click();
  }
  const asked = page.waitForRequest((request) =>
    new URL(request.url()).pathname.endsWith("/users/@me/activity"),
  );
  const filters = page.getByRole("navigation", { name: "your activity and saved messages" });
  await filters.getByText("Direct messages", { exact: true }).click();
  await expect(filters.getByRole("checkbox", { name: "Direct messages" })).not.toBeChecked();
  expect(new URL((await asked).url()).searchParams.get("filter[dms]")).toBe("false");
  await expect(feed.getByText("Did you get the photos?")).toHaveCount(0);
});
