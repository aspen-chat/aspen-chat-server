import { expect, test } from "@playwright/test";
import { lunchPoll, organiserRoleWith, pollQuestion, signInToWorld } from "./world";

/**
 * Closing a poll before its deadline: offered to its creator and to a holder of Manage
 * messages, which the organiser is given here by a role update, since Bob's poll is Bob's.
 */
test("a holder of Manage messages closes a poll early, and the card closes", async ({ page }) => {
  const publish = await signInToWorld(page);
  // The close answers as the server does: the poll closed now, told by its update and the
  // announcement.
  await page.route(new RegExp(`/api/v1/polls/${lunchPoll}/close$`), async (route) => {
    const closedAt = new Date().toISOString();
    publish({ serverEvent: "poll", type: "update", id: lunchPoll, closedAt });
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ id: lunchPoll, closedAt }),
    });
  });
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  const poll = page.getByRole("region", { name: `Poll: ${pollQuestion}` });
  await expect(poll).toBeVisible();
  // Bob's poll offers the organiser, who may not manage messages, no closing.
  await expect(poll.getByRole("button", { name: "Close poll" })).toHaveCount(0);
  publish(organiserRoleWith(["manageMessages"]));
  await poll.getByRole("button", { name: "Close poll" }).click();
  await expect(poll).toContainText("Closed");
  await expect(poll.getByRole("button", { name: "Close poll" })).toHaveCount(0);
});
