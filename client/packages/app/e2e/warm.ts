import { chromium, expect, type FullConfig } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Warms the dev server before any spec runs, so none meets it compiling. The dev server builds
 * each module the first time it is asked for, and may find a dependency to optimize and reload
 * the page as it does; and the attributions list (`attributions/plugin.ts`) runs `cargo
 * metadata` synchronously, holding up every other request until it is done, which in a fresh
 * checkout takes a minute or more. So the signed-in app and the attributions page are each loaded
 * once here, in one browser, and waited for.
 */
export default async function warm(config: FullConfig): Promise<void> {
  const baseURL = config.projects[0]?.use.baseURL;
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage(baseURL === undefined ? {} : { baseURL });
    await signInToWorld(page);
    await page.getByText("general", { exact: true }).first().click();
    await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible({
      timeout: WARM_TIMEOUT,
    });
    await page.goto("/attributions");
    await expect(page.getByRole("heading", { name: /^The app/, level: 2 })).toBeVisible({
      timeout: WARM_TIMEOUT,
    });
  } finally {
    await browser.close();
  }
}

/** How long a cold dev server is given to answer, `cargo metadata` fetching its index included. */
const WARM_TIMEOUT = 5 * 60_000;
