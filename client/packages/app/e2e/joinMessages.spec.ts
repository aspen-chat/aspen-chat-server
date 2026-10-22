import { expect, test } from "@playwright/test";
import { community, helper, roadmap, signInToWorld } from "./world";
import { message } from "./world/fixtures";

/**
 * A community's default channel and its join messages, against the stubbed world in
 * `world.ts`, whose organiser holds Manage community.
 */

test("the organiser chooses the default channel and turns join messages off", async ({ page }) => {
  const publish = await signInToWorld(page);
  const patches: unknown[] = [];
  await page.route(new RegExp(`/api/v1/communities/${community}$`), async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.fallback();
      return;
    }
    const patch = route.request().postDataJSON() as Record<string, unknown>;
    patches.push(patch);
    publish({ serverEvent: "community", type: "update", id: community, ...patch });
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ id: community, name: "Family", icon: null, ...patch }),
    });
  });
  await page.getByRole("button", { name: "Community settings" }).click();
  const dialog = page.getByRole("dialog", { name: /settings/ });
  const choice = dialog.getByRole("button", { name: /Default channel/ });
  await expect(choice).toContainText("# general");
  await choice.click();
  // Text channels alone are offered.
  await expect(page.getByRole("option", { name: "# Lounge" })).toHaveCount(0);
  await page.getByRole("option", { name: "# roadmap" }).click();
  await expect.poll(() => patches).toContainEqual({ defaultChannel: roadmap });
  await expect(choice).toContainText("# roadmap");
  const off = dialog.getByRole("checkbox", { name: /Don't post a message when someone joins/ });
  await expect(off).not.toBeChecked();
  await dialog.getByText("Don't post a message when someone joins").click();
  await expect.poll(() => patches).toContainEqual({ suppressJoinMessages: true });
  await expect(off).toBeChecked();
});

test("a join message names who joined, as a notice", async ({ page }) => {
  const publish = await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  publish({
    serverEvent: "message",
    type: "create",
    ...message(300, helper, "", 0, { kind: "memberJoined" }),
  });
  const notice = page.locator("article").filter({ hasText: "joined the community." });
  await expect(notice).toContainText("@Helper joined the community.");
});
