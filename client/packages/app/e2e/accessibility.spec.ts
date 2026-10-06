import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { PALETTES } from "../src/theme/palettes";
import { helper, signInToWorld, settleAnimations } from "./world";

/**
 * Every screen and dialog of the app, checked with axe against the WCAG rules it knows (ARIA,
 * names, landmarks, contrast), against the stubbed world in `world.ts`. Every palette is
 * checked in both colour schemes on one browser, and the default palette at more contrast too;
 * the rest check the default palette.
 */

/**
 * What axe finds wrong with the page as it stands, one line per element. With `popover`, a
 * popover is open, which React Aria renders at the end of the page outside every landmark, as
 * any floating layer must be; the landmark rule is skipped then, having checked the page
 * beneath in its other states, and every other rule covers the popover.
 */
async function problems(page: Page, where: string, popover: boolean): Promise<string[]> {
  await settleAnimations(page);
  // A tooltip is a floating layer too, and Firefox opens one for whatever a layout change
  // (a panel opening) slides under the resting pointer, once that has settled; the pointer
  // is taken off the page and any tooltip gone before the page is audited.
  await page.mouse.move(0, 0);
  await expect(page.locator('[role="tooltip"]')).toHaveCount(0);
  let builder = new AxeBuilder({ page });
  if (popover) {
    builder = builder.disableRules(["region"]);
  }
  const audit = await builder.analyze();
  return audit.violations.flatMap((violation) =>
    violation.nodes.map(
      (node) =>
        `${where}: ${violation.id} at ${node.target.join(" ")}: ${
          (node.failureSummary ?? "").split("\n")[1]?.trim() ?? violation.help
        }`,
    ),
  );
}

async function expectAccessible(page: Page, where: string, { popover = false } = {}) {
  expect.soft(await problems(page, where, popover)).toEqual([]);
}

/** Opens a channel from the list, going back to the list first on a phone. */
async function openChannel(page: Page, name: string) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText(name, { exact: true }).click();
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}

/** Every palette in both schemes, and the default one at more contrast in both. */
const combinations = [
  ...PALETTES.flatMap((palette) =>
    (["light", "dark"] as const).map((scheme) => ({ palette, scheme, contrast: "standard" })),
  ),
  ...(["light", "dark"] as const).map((scheme) => ({ palette: "aspen", scheme, contrast: "more" })),
];

for (const { palette, scheme, contrast } of combinations) {
  test.describe(`${palette}, ${scheme}${contrast === "more" ? ", more contrast" : ""}`, () => {
    test.beforeEach(async ({ page }, testInfo) => {
      test.skip(
        (palette !== "aspen" || scheme !== "light" || contrast !== "standard") &&
          testInfo.project.name !== "chromium",
        "every palette is checked on one browser",
      );
      await page.emulateMedia({ colorScheme: scheme });
      await page.addInitScript(
        ([chosen, level]) => {
          window.localStorage.setItem("aspen.palette", chosen);
          window.localStorage.setItem("aspen.contrast", level);
        },
        [palette, contrast] as const,
      );
      await signInToWorld(page);
    });

    test("channels, threads, and calls", async ({ page }) => {
      await expect(page.getByText("roadmap, unread")).toBeAttached();
      await expectAccessible(page, "channel list");
      await openChannel(page, "general");
      await expect(page.getByText("Sounds good.").first()).toBeVisible();
      await expectAccessible(page, "#general");
      // Below the large breakpoint the member list is a drawer over the channel.
      const showMembers = page.getByRole("button", { name: "Show members" });
      if (await showMembers.isVisible()) {
        await showMembers.click();
        await expect(page.getByRole("dialog", { name: "Members" })).toBeVisible();
        await expectAccessible(page, "members drawer");
        await page.keyboard.press("Escape");
        await expect(page.getByRole("dialog", { name: "Members" })).toBeHidden();
      }
      await page.getByRole("link", { name: /2 replies/ }).click();
      await expect(page.getByRole("heading", { name: "Thread" })).toBeVisible();
      await expectAccessible(page, "thread");
      await page.getByRole("button", { name: "Close thread" }).click();
      await openChannel(page, "Lounge");
      await expectAccessible(page, "voice channel");
    });

    test("direct messages", async ({ page }) => {
      await page
        .getByRole("navigation", { name: "Communities" })
        .getByRole("link", { name: /^Direct messages/ })
        .click();
      await expectAccessible(page, "DM list");
      await page
        .getByRole("link", { name: /^Bob With A Rather Long Display Name/ })
        .first()
        .click();
      await expect(page.getByText("Did you get the photos?")).toBeVisible();
      await expectAccessible(page, "DM");
    });

    test("profile cards, settings, and about", async ({ page }) => {
      await openChannel(page, "general");
      await page
        .locator("article")
        .filter({ hasText: "Sounds good." })
        .getByRole("button", { name: /^Show profile of Bob/ })
        .first()
        .click();
      await expect(page.getByRole("dialog", { name: /Bob/ })).toBeVisible();
      await expectAccessible(page, "profile card", { popover: true });
      await page.keyboard.press("Escape");
      const back = page.getByRole("link", { name: "Back to channels" });
      if (await back.isVisible()) {
        await back.click();
      }
      await page.getByRole("button", { name: "Settings", exact: true }).click();
      const settings = page.getByRole("dialog", { name: "Settings", exact: true });
      await settings.getByText("Developer mode", { exact: true }).last().click();
      await expectAccessible(page, "settings");
      await settings.getByRole("button", { name: "Your bots" }).click();
      await expect(page.getByRole("dialog", { name: "Your bots" })).toBeVisible();
      await expectAccessible(page, "bots");
      await page.keyboard.press("Escape");
      await settings.getByRole("button", { name: "About Aspen" }).click();
      await expect(
        page.getByRole("dialog", { name: "About Aspen" }).getByText(/^You're on /),
      ).toBeVisible();
      await expectAccessible(page, "about");
    });

    test("open source attributions", async ({ page }) => {
      await page.goto("/attributions");
      await expect(page.getByRole("heading", { name: /^The servers/, level: 2 })).toBeVisible({
        timeout: 60_000,
      });
      await page
        .getByRole("listitem")
        .filter({ hasText: /^react-aria-components/ })
        .getByRole("button", { name: "License text" })
        .click();
      await expectAccessible(page, "attributions");
    });

    test("message search", async ({ page }) => {
      await openChannel(page, "general");
      await page.getByRole("button", { name: "Search messages" }).click();
      const dialog = page.getByRole("dialog", { name: "Search messages" });
      await expectAccessible(page, "search");
      await dialog.getByRole("searchbox", { name: "Words" }).fill("lemonade");
      await dialog.getByRole("button", { name: "Search", exact: true }).click();
      await expect(dialog.getByRole("region", { name: "Results" }).getByRole("link")).toHaveCount(
        1,
      );
      await expectAccessible(page, "search results");
    });

    test("community settings", async ({ page }) => {
      await page.getByRole("button", { name: "Community settings" }).click();
      const dialog = page.getByRole("dialog");
      await expectAccessible(page, "community settings");
      await dialog.getByRole("tab", { name: "Roles" }).click();
      await expectAccessible(page, "roles");
      await dialog.getByRole("tab", { name: "Members" }).click();
      await expectAccessible(page, "members");
      await dialog.getByRole("tab", { name: "Emoji" }).click();
      await expect(dialog.getByRole("heading", { name: "Add an emoji" })).toBeVisible();
      await expectAccessible(page, "emoji");
    });

    test("the administration dashboard and a bot's page", async ({ page }) => {
      await page
        .getByRole("navigation", { name: "Communities" })
        .getByRole("link", { name: "Administration" })
        .click();
      await expect(page.getByRole("heading", { name: "Administration", level: 1 })).toBeVisible();
      const tabs = page.getByRole("navigation", { name: "Administration sections" });
      for (const tab of await tabs.getByRole("link").all()) {
        const name = await tab.innerText();
        await tab.click();
        await expect(tab).toHaveAttribute("aria-current", "page");
        await expect(page.getByRole("region").first()).toBeVisible();
        // Every table has loaded and faded back in: a table fades while its page loads.
        await expect
          .poll(() =>
            page
              .locator("table")
              .evaluateAll((tables) =>
                tables.every(
                  (table) =>
                    table.parentElement !== null &&
                    getComputedStyle(table.parentElement).opacity === "1",
                ),
              ),
          )
          .toBe(true);
        await expectAccessible(page, `administration: ${name}`);
      }
      await page.goto(`/bots/${helper}/add?permissions=sendMessages`);
      await expect(page.getByRole("heading", { name: "Add Helper to a community" })).toBeVisible();
      await expectAccessible(page, "adding a bot");
    });
  });
}
