import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { bob, general, signInToWorld, settleAnimations, submit } from "./world";

/**
 * Tagging, against the stubbed world in `world.ts`, where #roadmap holds two unread messages
 * that tag the caller and Bob's DM message tags them.
 */

const rail = (page: Page) => page.getByRole("navigation", { name: "Communities" });

test.beforeEach(async ({ page }) => {
  await signInToWorld(page);
});

test("unread tags are counted on the channel and its community", async ({ page }) => {
  await expect(page.getByText("roadmap, unread, 2 mentions")).toBeAttached();
  await expect(rail(page).getByRole("row", { name: "Family, unread, 2 mentions" })).toBeVisible();
});

test("a tag is a chip naming its person, and the message tagging the reader stands out", async ({
  page,
}) => {
  await rail(page)
    .getByRole("link", { name: /^Direct messages/ })
    .click();
  await page
    .getByRole("link", { name: /^Bob With A Rather Long Display Name/ })
    .first()
    .click();
  const tagged = page.locator("article").filter({ hasText: "Did you get the photos?" });
  await expect(tagged).toHaveAttribute("data-mentions-me", "true");
  await expect(tagged.getByRole("button", { name: "Show profile of Kate" }).last()).toHaveText(
    "@Kate",
  );
});

test("typing @ offers people to tag, and a pick is sent as a tag", async ({ page }) => {
  await page.getByText("general", { exact: true }).click();
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("hi @bo");
  await box.press("ArrowRight");
  const list = page.getByRole("listbox", { name: "People and roles to tag" });
  await expect(
    list.getByRole("option", { name: /Bob With A Rather Long Display Name/ }),
  ).toBeVisible();
  await box.press("Enter");
  await expect(box).toHaveValue("hi @bob ");
  await box.pressSequentially("see you");
  const sent = page.waitForRequest(
    (r) => r.method() === "POST" && r.url().endsWith(`/channels/${general}/messages`),
  );
  await submit(page);
  expect(((await sent).postDataJSON() as { content: string }).content).toBe(`hi <@${bob}> see you`);
});

test("a screen reader user tags someone from the keyboard alone", async ({ page }) => {
  await page.getByText("general", { exact: true }).click();
  const box = page.getByRole("textbox", { name: "Message" });
  await box.focus();
  await page.keyboard.type("thanks @");

  // Suggestions are announced, with how to use them.
  await expect(page.getByRole("status")).toHaveText(
    /^\d+ suggestions? to tag\. .*Enter or Tab tag/,
  );
  // The box names the list it controls, and which option is chosen, as focus stays in it.
  const listId = await box.getAttribute("aria-controls");
  expect(listId).not.toBeNull();
  const list = page.locator(`[id="${listId ?? ""}"]`);
  await expect(list).toHaveRole("listbox");
  await expect(list).toHaveAccessibleName("People and roles to tag");
  // The message box and its suggestions break no rule of ARIA or of accessible naming.
  await settleAnimations(page);
  const audit = await new AxeBuilder({ page }).include("form:has(textarea)").analyze();
  expect(
    audit.violations.flatMap((v) =>
      v.nodes.map((n) => `${v.id}: ${n.target.join(" ")} ${n.failureSummary ?? ""}`),
    ),
  ).toEqual([]);
  /** The chosen option's name, as the accessibility tree gives it to a screen reader. */
  const chosenName = async () => {
    const id = await box.getAttribute("aria-activedescendant");
    const option = page.locator(`[id="${id ?? ""}"]`);
    await expect(option).toHaveRole("option");
    await expect(option).toHaveAttribute("aria-selected", "true");
    const snapshot = await option.ariaSnapshot();
    return /^- option "([^"]*)"/.exec(snapshot)?.[1] ?? snapshot;
  };

  // Down the list to Bob, hearing each name on the way.
  const heard: string[] = [];
  for (let step = 0; step < 8; step += 1) {
    heard.push(await chosenName());
    if (heard.at(-1)?.startsWith("Bob") === true) {
      break;
    }
    await page.keyboard.press("ArrowDown");
  }
  expect(heard.at(-1)).toBe("Bob With A Rather Long Display Name, @bob");
  // Every name heard is a person's or a role's, not the pictures beside them.
  for (const name of heard) {
    expect(name).toMatch(/^[^,]+, (@\S+|Role|Everyone here)$/);
  }
  await expect(box).toBeFocused();

  await page.keyboard.press("Enter");
  // Picked: the box reads the tag back as written, and the list is gone.
  await expect(box).toHaveValue("thanks @bob ");
  await expect(box).not.toHaveAttribute("aria-controls");
  await expect(box).not.toHaveAttribute("aria-activedescendant");
  await expect(page.getByRole("status")).toHaveText("");

  await page.keyboard.type("for the photos");
  await submit(page);
  // The message names Bob by his name, as a control that opens his card.
  const sent = page.locator("article").filter({ hasText: "for the photos" });
  await expect(
    sent.getByRole("button", { name: "Show profile of Bob With A Rather Long Display Name" }),
  ).toHaveText("@Bob With A Rather Long Display Name");
});

test("someone beyond the member sample is found to tag by searching", async ({ page }) => {
  await page.getByText("general", { exact: true }).click();
  const box = page.getByRole("textbox", { name: "Message" });
  await box.focus();
  await page.keyboard.type("@dan");
  const dana = page
    .getByRole("listbox", { name: "People and roles to tag" })
    .getByRole("option", { name: "Dana From Far Away, @dana" });
  await expect(dana).toBeVisible();
  await dana.click();
  await expect(box).toHaveValue("@dana ");
});
