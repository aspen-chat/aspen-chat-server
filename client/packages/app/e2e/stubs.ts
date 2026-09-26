import type { Page } from "@playwright/test";

export const uuid = "0190f0a0-0000-7000-8000-000000000001";

/**
 * Stubs everything the app touches after signing in, so a spec never depends on a backend
 * behind the dev server's proxy: the caller's profile, an empty community list, the token
 * refresh, and the event stream, which answers `identify` with `ready` and then stays silent.
 */
export async function stubSignedInBackend(page: Page, name: string): Promise<void> {
  await page.route(/\/api\/v1\/users\/%40me$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ id: uuid, name, icon: null, onlineStatus: "online" }),
    }),
  );
  await page.route(/\/api\/v1\/users\/%40me\/communities\?/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ data: [], included: {} }),
    }),
  );
  await page.route("**/api/v1/auth/token-refresh", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        sessionToken: "s",
        sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
      }),
    }),
  );
  await page.routeWebSocket(/\/api\/v1\/events$/, (ws) => {
    ws.onMessage((message) => {
      const frame: unknown = JSON.parse(String(message));
      if (
        typeof frame === "object" &&
        frame !== null &&
        (frame as { type?: unknown }).type === "identify"
      ) {
        ws.send(JSON.stringify({ type: "ready", userId: uuid, resumed: false }));
      }
    });
  });
}
