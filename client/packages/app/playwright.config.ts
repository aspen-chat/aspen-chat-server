import { defineConfig, devices } from "@playwright/test";

/**
 * Browser matrix for the web target: Chromium, Firefox, and WebKit (Safari's engine). Run
 * `pnpm e2e:install` once to download the browsers.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: process.env["CI"] !== undefined,
  retries: process.env["CI"] !== undefined ? 2 : 0,
  reporter: process.env["CI"] !== undefined ? "github" : "list",
  use: {
    baseURL: "http://localhost:5173",
    trace: "on-first-retry",
  },
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:5173",
    reuseExistingServer: process.env["CI"] === undefined,
    timeout: 60_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
});
