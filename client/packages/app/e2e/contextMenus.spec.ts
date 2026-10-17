import { expect, test, type Page } from "@playwright/test";
import { bob, lounge, ownText, reactedText, signInToWorld, type Publish } from "./world";

/**
 * Right-click menus, against the stubbed world in `world.ts`: a message's actions by name, with
 * what was right-clicked inside it first.
 */

const message = (page: Page) => page.locator("article").filter({ hasText: reactedText }).last();
const messageMenu = (page: Page) => page.getByRole("menu", { name: "Message actions" });

let publish: Publish;

test.beforeEach(async ({ page, isMobile }) => {
  test.skip(isMobile, "A touch screen offers these by long presses and buttons.");
  publish = await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(message(page)).toBeVisible();
});

test("right-clicking a message offers its actions by name", async ({ page }) => {
  await message(page).getByText(reactedText).click({ button: "right" });
  const menu = messageMenu(page);
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Copy text" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Save" })).toBeVisible();
  await page.waitForTimeout(500);
  await page.screenshot({ path: "test-results/message-menu.png" });
  await menu.getByRole("menuitem", { name: "Copy text" }).click();
  await expect(menu).toBeHidden();
});

const bobsName = "Bob With A Rather Long Display Name";
const userMenu = (page: Page) => page.getByRole("menu", { name: `Actions for ${bobsName}` });

test("right-clicking someone's name offers what can be done to them", async ({ page }) => {
  await message(page)
    .getByRole("button", { name: `Show profile of ${bobsName}` })
    .last()
    .click({ button: "right" });
  const menu = userMenu(page);
  await expect(menu).toBeVisible();
  // Their menu, not the message's.
  await expect(messageMenu(page)).toHaveCount(0);
  for (const item of ["Profile", "Message", "Call", "Mention", "Copy username", "Block"]) {
    await expect(menu.getByRole("menuitem", { name: item, exact: true })).toBeVisible();
  }
  await page.waitForTimeout(500);
  await page.screenshot({ path: "test-results/user-menu.png" });
  await menu.getByRole("menuitem", { name: "Mention", exact: true }).click();
  await expect(page.getByRole("textbox", { name: /Message/ })).toHaveValue("@bob ");
});

test("someone the reader may moderate has moderation in their menu", async ({ page }) => {
  await page
    .getByRole("complementary", { name: "Members" })
    .getByRole("button", { name: /^Show profile of Helper/ })
    .click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Actions for Helper" });
  for (const item of ["Remove from community", "Ban from community"]) {
    await expect(menu.getByRole("menuitem", { name: item })).toBeVisible();
  }
  // The one role there is ranks with the reader's own, so they may give none.
  await expect(menu.getByRole("menuitem", { name: "Roles" })).toHaveCount(0);
  await menu.getByRole("menuitem", { name: "Remove from community" }).click();
  await expect(page.getByRole("alertdialog", { name: "Remove Helper?" })).toBeVisible();
});

test("someone in a call has the call's controls atop their menu", async ({ page }) => {
  const session = "0190f0a0-0000-7000-8000-0000000000c1";
  publish({
    serverEvent: "voiceSession",
    type: "create",
    id: session,
    channel: lounge,
    voiceServer: "0190f0a0-0000-7000-8000-0000000000c2",
    createdAt: new Date().toISOString(),
  });
  publish({
    serverEvent: "voiceParticipant",
    type: "create",
    session,
    channel: lounge,
    user: bob,
    joinedAt: new Date().toISOString(),
    muted: false,
    deafened: false,
    sharingScreen: false,
  });
  const participant = page.locator(`[data-voice-user="${bob}"]`);
  await participant.click({ button: "right" });
  const menu = page.getByRole("dialog", { name: `Actions for ${bobsName}` });
  await expect(menu.getByRole("slider", { name: "Volume" })).toBeVisible();
  for (const item of ["Mute for me", "Server mute", "Remove from call", "Profile", "Message"]) {
    await expect(menu.getByRole("menuitem", { name: item, exact: true })).toBeVisible();
  }
  // The card's own menu is not offered as well.
  await expect(page.getByRole("menu")).toHaveCount(1);
  await page.waitForTimeout(500);
  await page.screenshot({ path: "test-results/participant-menu.png" });
});

test("a community's name opens its settings, by a click or a right click", async ({ page }) => {
  const name = page.getByRole("heading", { level: 1, name: "Family" }).getByRole("button");
  const settings = page.getByRole("dialog", { name: /Family/ });
  await name.click();
  await expect(settings).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(settings).toBeHidden();
  await name.click({ button: "right" });
  await expect(settings).toBeVisible();
});

test("a community's icon on the rail offers what can be done to it", async ({ page }) => {
  const rail = page.getByRole("navigation", { name: "Communities" });
  await rail.getByRole("row", { name: "Family, unread" }).locator("div").first().click({
    button: "right",
  });
  const menu = page.getByRole("menu", { name: "Options for Family" });
  for (const item of ["Notifications", "Invite people", "Community settings", "Copy link"]) {
    await expect(menu.getByRole("menuitem", { name: item })).toBeVisible();
  }
  await menu.getByRole("menuitem", { name: "Mark as read" }).click();
  await expect(rail.getByRole("row", { name: "Family", exact: true })).toBeVisible();
  // Choosing from the menu did not also open the community, or anything else, behind it.
  await rail.getByRole("row", { name: "Family", exact: true }).locator("div").first().click({
    button: "right",
  });
  await expect(menu.getByRole("menuitem", { name: "Mark as read" })).toHaveCount(0);
  await menu.getByRole("menuitem", { name: "Community settings" }).click();
  await expect(page.getByRole("dialog", { name: /Family settings/ })).toBeVisible();
});

test("a category's heading offers what can be done to it", async ({ page }) => {
  const heading = page.getByRole("heading", { level: 2, name: "Planning" });
  await heading.click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Options for Planning" });
  for (const item of ["Collapse category", "Add a channel", "Rename category", "Delete category"]) {
    await expect(menu.getByRole("menuitem", { name: item })).toBeVisible();
  }
  await menu.getByRole("menuitem", { name: "Mark as read" }).click();
  await expect(page.getByText("roadmap, unread")).toHaveCount(0);
  await heading.click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Collapse category" }).click();
  await expect(heading.getByRole("button")).toHaveAttribute("aria-expanded", "false");
});

test("the reader's own name opens their status menu", async ({ page }) => {
  await page.getByRole("button", { name: /^Change your status/ }).click({ button: "right" });
  await expect(page.getByRole("dialog", { name: "Your status" })).toBeVisible();
});

test("a saved message's row offers going to it and removing it", async ({ page }) => {
  await message(page).hover();
  await message(page).getByRole("button", { name: "Save message" }).click();
  await page.goto("/saved");
  const list = page.getByRole("list", { name: "Saved messages" });
  await list.getByText(reactedText).click({ button: "right" });
  const menu = messageMenu(page);
  for (const item of ["Go to message", "Copy text", "Copy link"]) {
    await expect(menu.getByRole("menuitem", { name: item })).toBeVisible();
  }
  await menu.getByRole("menuitem", { name: "Remove from saved" }).click();
  await expect(page.getByText(/^Nothing saved yet/)).toBeVisible();
});

test("the keyboard opens a message's menu at the message, its first item ready", async ({
  page,
  browserName,
}) => {
  test.skip(browserName !== "chromium", "Only Chromium makes a menu of a key Playwright presses.");
  await message(page).focus();
  await page.keyboard.press("Shift+F10");
  const menu = messageMenu(page);
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
});

test("right-clicking something else while a menu is open opens that one's instead", async ({
  page,
}) => {
  await message(page).getByText(reactedText).click({ button: "right" });
  await expect(messageMenu(page)).toBeVisible();
  await page
    .getByRole("heading", { level: 2, name: "Planning" })
    .click({ button: "right", force: true });
  await expect(page.getByRole("menu", { name: "Options for Planning" })).toBeVisible();
  await expect(messageMenu(page)).toHaveCount(0);
});

test("a voice channel's row offers joining its call", async ({ page }) => {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("Lounge", { exact: true })
    .click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Options for Lounge" });
  for (const item of ["Join the call", "Join muted", "Copy link"]) {
    await expect(menu.getByRole("menuitem", { name: item })).toBeVisible();
  }
});

test("a link right-clicked in a message is offered first", async ({ page }) => {
  const link = page.getByRole("link", { name: /missing-picture/ }).last();
  await link.scrollIntoViewIfNeeded();
  await link.click({ button: "right" });
  const menu = messageMenu(page);
  await expect(menu.getByRole("menuitem").first()).toHaveText("Open link");
  await expect(menu.getByRole("menuitem", { name: "Copy link address" })).toBeVisible();
  // The message's own actions follow.
  await expect(menu.getByRole("menuitem", { name: "Copy text" })).toBeVisible();
});

test("editing the reader's own message from its menu puts the editor in its place", async ({
  page,
}) => {
  const own = page.locator("article").filter({ hasText: ownText }).last();
  await own.getByText(ownText).click({ button: "right" });
  await messageMenu(page).getByRole("menuitem", { name: "Edit message" }).click();
  await expect(own.getByRole("textbox")).toBeFocused();
});
