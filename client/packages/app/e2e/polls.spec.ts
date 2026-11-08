import { expect, test, type Page } from "@playwright/test";
import { general, pollQuestion, signInToWorld } from "./world";

/**
 * Written-in poll answers, against Bob's poll in the stubbed world of `world.ts`: it lists
 * Bob's answer with a note naming him, takes one of the caller's own, and lets them remove it.
 */

async function openPoll(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  const poll = page.getByRole("region", { name: `Poll: ${pollQuestion}` });
  await expect(poll).toBeVisible();
  return poll;
}

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("a written-in answer is offered with a note naming who wrote it in", async ({ page }) => {
  const poll = await openPoll(page);
  const pancakes = poll.getByRole("button", { name: "Vote for Pancakes" });
  await expect(pancakes).toContainText("Written in by Bob With A Rather Long Display Name");
  // Bob's own answer is his and the poll is his too; the caller may only vote for it.
  await expect(poll.getByRole("button", { name: /^Remove the answer/ })).toHaveCount(0);
});

test("the caller writes in an answer, votes for it, and can remove it", async ({ page }) => {
  const poll = await openPoll(page);
  const field = poll.getByRole("textbox", { name: "Add your own answer" });
  await field.fill("Tacos");
  await poll.getByRole("button", { name: "Add", exact: true }).click();

  const tacos = poll.getByRole("button", { name: "Withdraw your vote for Tacos" });
  await expect(tacos).toContainText("Written in by Kate");
  // One answer each: the field goes once the caller has one standing.
  await expect(field).toHaveCount(0);

  await poll.getByRole("button", { name: "Remove the answer Tacos" }).click();
  const confirm = page.getByRole("alertdialog");
  await expect(confirm).toContainText("“Tacos” and every vote for it will be taken off the poll.");
  await confirm.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(tacos).toHaveCount(0);
  await expect(poll.getByRole("textbox", { name: "Add your own answer" })).toBeVisible();
});

test("creating a poll leaves what the message box holds unsent", async ({ page }) => {
  await openPoll(page);
  // The channel is busy, so the poll waits its turn: the server answers with what it holds.
  await page.route(/\/api\/v1\/channels\/[^/]+\/polls$/, (route) =>
    route.fulfill({
      status: 202,
      json: {
        id: "0190f0a0-0000-7000-8004-000000000001",
        channelId: general,
        content: "Tea?",
        attachments: [],
        echoToParent: false,
        replyTo: null,
        heldAt: new Date().toISOString(),
        waiting: "mana",
        kind: "poll",
        target: null,
      },
    }),
  );
  const sent: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && /\/channels\/[^/]+\/messages$/.test(request.url())) {
      sent.push(request.url());
    }
  });
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("still writing this");
  // Its own button on a wide screen, an item of the + menu on a narrow one.
  const button = page.getByRole("button", { name: "Create a new poll" });
  if (await button.isVisible()) {
    await button.click();
  } else {
    await page
      .getByRole("button", { name: "Add a file or a poll, or schedule this message" })
      .click();
    await page.getByRole("menuitem", { name: "Create a new poll" }).click();
  }
  const dialog = page.getByRole("dialog", { name: "New poll" });
  await dialog.getByLabel("Question").fill("Tea?");
  await dialog.getByPlaceholder("Option 1").fill("Yes");
  await dialog.getByPlaceholder("Option 2").fill("No");
  await dialog.getByRole("button", { name: "Create a poll" }).click();
  await expect(dialog).toBeHidden();
  // The poll's form is drawn apart from the box, and submitting it sends nothing of the box's.
  await expect(box).toHaveValue("still writing this");
  expect(sent).toEqual([]);
});
