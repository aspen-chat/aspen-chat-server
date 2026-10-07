import { expect, test, type Page } from "@playwright/test";
import { bob, general, helper, me, signInToWorld, type Publish } from "./world";

/**
 * Who is typing, against the stubbed world: others shown above the message box on a line kept
 * for them, never the reader, and gone when they stop or the connection drops; and the box
 * telling the stream as the reader writes, unless they turned it off.
 */

async function openGeneral(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText("general", { exact: true }).click();
}

const box = (page: Page) => page.getByRole("textbox", { name: "Message" });

function typing(publish: Publish, userId: string, on = true) {
  publish.ephemeral?.({ type: "typing", channelId: general, userId, typing: on });
}

function sentTyping(publish: Publish) {
  return (publish.sent ?? []).filter((f) => f.type === "typing" || f.type === "stoppedTyping");
}

test("others typing are named above the box, which never moves for them", async ({ page }) => {
  const publish = await signInToWorld(page);
  await openGeneral(page);
  const top = async () => (await box(page).boundingBox())?.y;
  const before = await top();
  typing(publish, bob);
  const line = page.getByText(/is typing…$/);
  await expect(line).toHaveText("Bob With A Rather Long Display Name is typing…");
  expect(await top()).toBe(before);
  typing(publish, helper);
  await expect(page.getByText(/are typing…$/)).toHaveText(
    "Bob With A Rather Long Display Name and Helper are typing…",
  );
  // The reader is never told of themself.
  typing(publish, me);
  await expect(page.getByText(/are typing…$/)).toHaveText(
    "Bob With A Rather Long Display Name and Helper are typing…",
  );
  for (const n of [1, 2]) {
    typing(publish, `0190f0a0-0000-7000-8000-0000000009${String(n).padStart(2, "0")}`);
  }
  await expect(page.getByText("Several people are typing…")).toBeVisible();
  for (const n of [1, 2]) {
    typing(publish, `0190f0a0-0000-7000-8000-0000000009${String(n).padStart(2, "0")}`, false);
  }
  typing(publish, helper, false);
  await expect(line).toHaveText("Bob With A Rather Long Display Name is typing…");
  // A connection lost forgets who was typing, since nobody can say they stopped.
  publish.drop?.();
  await expect(page.getByText(/typing…$/)).toHaveCount(0);
  expect(await top()).toBe(before);
});

test("writing tells the stream, and stops telling it once turned off", async ({ page }) => {
  const publish = await signInToWorld(page);
  await openGeneral(page);
  await box(page).fill("hello");
  await expect.poll(() => sentTyping(publish)).toEqual([{ type: "typing", channelId: general }]);
  await box(page).fill("");
  await expect
    .poll(() => sentTyping(publish).at(-1))
    .toEqual({ type: "stoppedTyping", channelId: general });
  // Settings are beside the channel list, which a phone shows in the conversation's place.
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByText("Show others when I'm typing", { exact: true }).click();
  await page.keyboard.press("Escape");
  await openGeneral(page);
  const told = sentTyping(publish).length;
  await box(page).fill("quietly");
  await page.waitForTimeout(300);
  expect(sentTyping(publish)).toHaveLength(told);
});
