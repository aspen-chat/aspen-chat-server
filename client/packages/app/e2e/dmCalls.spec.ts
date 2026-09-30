import { expect, test, type Page } from "@playwright/test";
import { bob, dm, me, signInToWorld } from "./world";

/**
 * Calls in DMs, against the stubbed world in `world.ts`, where the caller has a DM with Bob.
 * The world has no voice server, so these check what a call under way shows; joining one is
 * checked against real servers by hand.
 */

const session = "0190f0a0-0000-7000-8000-000000000071";
const bobName = "Bob With A Rather Long Display Name";

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test("a call under way in a DM shows on its row and above its messages", async ({ page }) => {
  const publish = await signInToWorld(page);
  await rail(page)
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  const row = page.getByRole("navigation", { name: "Direct messages" }).getByRole("link", {
    name: new RegExp(bobName),
  });
  await expect(row.getByRole("img", { name: "Call under way" })).toHaveCount(0);

  publish({
    serverEvent: "voiceSession",
    type: "create",
    id: session,
    channel: dm,
    voiceServer: "0190f0a0-0000-7000-8000-000000000072",
    createdAt: new Date().toISOString(),
  });
  publish({
    serverEvent: "voiceParticipant",
    type: "create",
    session,
    channel: dm,
    user: bob,
    joinedAt: new Date().toISOString(),
    muted: false,
    deafened: false,
    sharingScreen: false,
  });
  await expect(row.getByRole("img", { name: "Call under way" })).toBeVisible();
  await row.click();
  const call = page.getByRole("region", { name: "Call", exact: true });
  await expect(call.locator(`[data-voice-tile="${bob}"]`)).toBeVisible();
  await expect(call.getByRole("button", { name: "Join the call" })).toBeVisible();
  // The header offers the same.
  await expect(page.locator("header").getByRole("button", { name: "Join the call" })).toBeVisible();
});

test("a person's card offers to call them, and a bot's does not", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name.startsWith("phone"), "a phone shows no member list");
  await signInToWorld(page);
  const members = page.getByRole("complementary", { name: "Members" });
  await members.getByRole("button", { name: `Show profile of ${bobName}` }).click();
  const card = page.getByRole("dialog", { name: `Profile of ${bobName}` });
  await card.getByRole("button", { name: `Call ${bobName}` }).hover();
  await expect(page.getByRole("tooltip")).toHaveText(`Call ${bobName}`);
  // The first Escape closes the tooltip, the second the card.
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await expect(card).toHaveCount(0);
  await members.getByRole("button", { name: "Show profile of Helper" }).click();
  await expect(
    page.getByRole("dialog", { name: "Profile of Helper" }).getByRole("button", { name: /^Call / }),
  ).toHaveCount(0);
});

/**
 * Opens the DM list, by which time the event stream is up: events published before it is are
 * lost.
 */
async function openDms(page: Page) {
  await rail(page)
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await expect(page.getByRole("navigation", { name: "Direct messages" })).toBeVisible();
}

/** Starts a call in the DM with Bob, as its server announces one Bob started. */
function bobCalls(
  publish: Awaited<ReturnType<typeof signInToWorld>>,
  { untilMs = 15_000, ringMe = true }: { untilMs?: number; ringMe?: boolean } = {},
) {
  publish({
    serverEvent: "voiceSession",
    type: "create",
    id: session,
    channel: dm,
    voiceServer: "0190f0a0-0000-7000-8000-000000000072",
    createdAt: new Date().toISOString(),
  });
  publish({
    serverEvent: "voiceParticipant",
    type: "create",
    session,
    channel: dm,
    user: bob,
    joinedAt: new Date().toISOString(),
    muted: false,
    deafened: false,
    sharingScreen: false,
  });
  if (ringMe) {
    publish({
      serverEvent: "voiceRing",
      type: "create",
      session,
      user: me,
      channel: dm,
      caller: bob,
      until: new Date(Date.now() + untilMs).toISOString(),
    });
  }
}

test("a call ringing the user asks them to accept or decline, and declining tells the server", async ({
  page,
}) => {
  const declined: string[] = [];
  const publish = await signInToWorld(page, async (p) => {
    await p.route(new RegExp(`/channels/${dm}/voice/rings/(%40|@)me$`), (route) => {
      declined.push(route.request().method());
      publish({ serverEvent: "voiceRing", type: "delete", session, user: me });
      return route.fulfill({ status: 204 });
    });
  });
  await openDms(page);
  bobCalls(publish);
  const modal = page.getByRole("alertdialog", { name: `${bobName} is calling` });
  await expect(modal).toBeVisible();
  await expect(modal.getByText("Direct message")).toBeVisible();
  await modal.getByRole("button", { name: "Decline" }).click();
  await expect(modal).toHaveCount(0);
  expect(declined).toEqual(["DELETE"]);
});

test("a ring nobody answers ends by itself at its time", async ({ page }) => {
  const publish = await signInToWorld(page);
  await openDms(page);
  bobCalls(publish, { untilMs: 2_000 });
  const modal = page.getByRole("alertdialog", { name: `${bobName} is calling` });
  await expect(modal).toBeVisible();
  await expect(modal).toHaveCount(0, { timeout: 5_000 });
});

test("someone being rung shows darkened in the call, and a call that ended says how long it lasted", async ({
  page,
}) => {
  const publish = await signInToWorld(page);
  await rail(page)
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await page
    .getByRole("navigation", { name: "Direct messages" })
    .getByRole("link", { name: new RegExp(bobName) })
    .click();
  // Bob's call rings Helper here, standing in for anyone else in a group DM.
  bobCalls(publish, { ringMe: false });
  publish({
    serverEvent: "voiceRing",
    type: "create",
    session,
    user: "0190f0a0-0000-7000-8000-000000000003",
    channel: dm,
    caller: bob,
    until: new Date(Date.now() + 15_000).toISOString(),
  });
  const rung = page.locator('[data-ringing="0190f0a0-0000-7000-8000-000000000003"]');
  await expect(rung).toBeVisible();
  await expect(rung).toHaveCSS("opacity", "1");
  await expect(rung).toHaveCSS("filter", "brightness(0.75)");
  publish({
    serverEvent: "message",
    type: "create",
    id: "0190f0a0-0000-7000-8000-0000000000f1",
    channelId: dm,
    author: bob,
    content: "",
    timestamp: new Date().toISOString(),
    editedAt: null,
    attachments: [],
    linkPreviews: [],
    kind: "call",
    poll: null,
    thread: null,
    echoOf: null,
    mentions: { users: [], roles: [], everyone: false },
    callSeconds: 125,
  });
  await expect(
    page.getByText(`${bobName} started a call that lasted 2 minutes and 5 seconds.`),
  ).toBeVisible();
});
