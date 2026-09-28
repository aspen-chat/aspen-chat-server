import { defineConfig, devices } from "@playwright/test";

/**
 * Browser matrix for the web target: Chromium, Firefox, and WebKit (Safari's engine) at desktop
 * size, and two phones, an Android one on Chromium and an iPhone on WebKit, which also run the
 * phone-only `mobile.spec.ts`. Run `pnpm e2e:install` once to download the browsers.
 *
 * The specs start the dev server on `E2E_PORT` (5173 by default), or use one already running
 * there outside CI.
 */
const port = process.env["E2E_PORT"] ?? "5173";
const phoneOnly = /mobile\.spec\.ts$/;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  // Every worker runs its own browser. Playwright's default, half the cores, starts a dozen or
  // more on a large workstation, which with a desktop's own programs can exhaust its memory,
  // so a run outside CI keeps to a few.
  ...(process.env["CI"] === undefined ? { workers: 4 } : {}),
  forbidOnly: process.env["CI"] !== undefined,
  retries: process.env["CI"] !== undefined ? 2 : 0,
  reporter: process.env["CI"] !== undefined ? "github" : "list",
  use: {
    baseURL: `http://localhost:${port}`,
    trace: "on-first-retry",
  },
  webServer: {
    command: `pnpm dev --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: process.env["CI"] === undefined,
    timeout: 60_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] }, testIgnore: phoneOnly },
    { name: "firefox", use: { ...devices["Desktop Firefox"] }, testIgnore: phoneOnly },
    { name: "webkit", use: { ...devices["Desktop Safari"] }, testIgnore: phoneOnly },
    { name: "phone-chromium", use: { ...devices["Pixel 7"] } },
    { name: "phone-webkit", use: { ...devices["iPhone 14"] } },
  ],
});
