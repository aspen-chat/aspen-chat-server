import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { settleAnimations, signInToWorld } from "./world";

/**
 * The desktop app updating itself, stood in for by its bridge (`packages/desktop/src/main/updates.ts`
 * is the real one): Settings offers the tracks and keeps the choice in the main process, an
 * update that is ready shows a strip that restarts into it, and updating turned off from outside
 * the app is said rather than offered. A browser shows none of it.
 */

declare global {
  interface Window {
    updatesChosen: string[];
    updatesRestarted: number;
    /** Tells every listener of a new state, as the main process does. */
    updatesHeard: (state: DesktopUpdateState) => void;
  }
}

/**
 * Stands in for the Windows build that updates itself, whose updating starts at `first`. A
 * desktop shell signs in to a server it has been given, as the dev server is here.
 */
async function standInForUpdatingDesktop(
  page: Page,
  baseURL: string | undefined,
  first: DesktopUpdateState,
) {
  const server = new URL(baseURL ?? "").origin;
  await page.addInitScript(
    ({ state, server }) => {
      localStorage.setItem("aspen.serverUrl", server);
      let current = state;
      const listeners = new Set<(state: DesktopUpdateState) => void>();
      window.updatesChosen = [];
      window.updatesRestarted = 0;
      window.updatesHeard = (next) => {
        current = next;
        for (const listener of listeners) {
          listener(next);
        }
      };
      (window as { aspenDesktop?: unknown }).aspenDesktop = {
        platform: "win32",
        versions: { electron: "0", chrome: "0" },
        // What the page asks of every desktop shell as it starts.
        zoom: {
          get: () => Promise.resolve(1),
          set: () => Promise.resolve(),
          onStep: () => () => undefined,
        },
        titleBar: {
          paint: () => undefined,
          choice: () => Promise.resolve(null),
          choose: () => Promise.resolve(),
          relaunch: () => undefined,
        },
        updates: {
          state: () => Promise.resolve(current),
          choose: (choice: string) => {
            window.updatesChosen.push(choice);
            window.updatesHeard({ ...current, choice: choice as DesktopUpdateChoice });
            return Promise.resolve();
          },
          restart: () => {
            window.updatesRestarted += 1;
          },
          onChange: (listener: (state: DesktopUpdateState) => void) => {
            listeners.add(listener);
            return () => {
              listeners.delete(listener);
            };
          },
        },
      };
    },
    { state: first, server },
  );
}

async function openSettings(page: Page) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  return page.getByRole("dialog", { name: "Settings", exact: true });
}

const idle: DesktopUpdateState = { choice: "stable", locked: null, status: "idle", version: null };

test("a browser offers no updates of its own", async ({ page }) => {
  await signInToWorld(page);
  const settings = await openSettings(page);
  await expect(settings.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(settings.getByRole("heading", { name: "Updates" })).toHaveCount(0);
});

test("the desktop app follows the track chosen in Settings", async ({ page, baseURL }) => {
  await standInForUpdatingDesktop(page, baseURL, idle);
  await signInToWorld(page);
  const settings = await openSettings(page);
  const tracks = settings.getByRole("radiogroup", { name: "Update to" });
  await expect(tracks.getByRole("radio", { name: /^Releases/ })).toBeChecked();

  await tracks.getByText("Betas and releases", { exact: true }).click();
  await expect(tracks.getByRole("radio", { name: /^Betas and releases/ })).toBeChecked();
  await tracks.getByText("Nothing", { exact: true }).click();
  await expect(tracks.getByRole("radio", { name: /^Nothing/ })).toBeChecked();
  expect(await page.evaluate(() => window.updatesChosen)).toEqual(["beta", "off"]);

  await settleAnimations(page);
  await page.mouse.move(0, 0);
  const audit = await new AxeBuilder({ page }).include('[role="dialog"]').analyze();
  expect(
    audit.violations.flatMap((v) => v.nodes.map((n) => `${v.id} at ${n.target.join(" ")}`)),
  ).toEqual([]);
});

test("an update that is ready offers to restart into it, from the strip and from Settings", async ({
  page,
  baseURL,
}) => {
  await standInForUpdatingDesktop(page, baseURL, idle);
  await signInToWorld(page);
  const strip = page.getByRole("status").filter({ hasText: "Aspen 0.3.0 is ready." });
  await expect(strip).toHaveCount(0);

  await page.evaluate(() => {
    window.updatesHeard({ choice: "stable", locked: null, status: "ready", version: "0.3.0" });
  });
  await expect(strip).toHaveText(/It installs when you quit Aspen\./);
  await strip.getByRole("button", { name: "Restart now" }).click();
  expect(await page.evaluate(() => window.updatesRestarted)).toBe(1);

  const settings = await openSettings(page);
  await settings.getByRole("button", { name: "Restart now" }).click();
  expect(await page.evaluate(() => window.updatesRestarted)).toBe(2);
});

test("updating turned off from outside the app is said, not offered", async ({ page, baseURL }) => {
  await standInForUpdatingDesktop(page, baseURL, { ...idle, locked: "administrator" });
  await signInToWorld(page);
  const settings = await openSettings(page);
  await expect(settings.getByText(/Your administrator has turned off updates/)).toBeVisible();
  await expect(settings.getByRole("radiogroup", { name: "Update to" })).toHaveCount(0);

  await page.evaluate(() => {
    window.updatesHeard({ choice: "stable", locked: "installer", status: "idle", version: null });
  });
  await expect(settings.getByText(/without \/DISABLEUPDATES/)).toBeVisible();
});
