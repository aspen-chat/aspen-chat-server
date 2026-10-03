import type { Page } from "@playwright/test";

export const uuid = "0190f0a0-0000-7000-8000-000000000001";

/** A completed sign-in, as `POST /auth/login` and the second-factor step answer it. */
export function signedIn(status: "signedIn" | null = "signedIn"): string {
  return JSON.stringify({
    ...(status === null ? {} : { status }),
    userId: uuid,
    refreshToken: "r",
    sessionToken: "s",
    sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
    twoFactorEnrollmentRequired: false,
  });
}

/**
 * A server that offers passwords only, so the sign-in screen shows no passkey button, takes
 * new accounts from anyone unless `registrationInviteRequired`, and has no display name or icon.
 */
export async function stubAuthMethods(
  page: Page,
  { registrationInviteRequired = false }: { registrationInviteRequired?: boolean } = {},
): Promise<void> {
  await stubDeploymentProfile(page, { displayName: null, icon: null });
  await page.route("**/api/v1/auth/methods", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        passkeys: null,
        twoFactorRequired: false,
        registrationInviteRequired,
      }),
    }),
  );
}

/**
 * How the server presents itself on the sign-in screen (`GET /deployment`). Registered after
 * `stubAuthMethods`, it replaces the empty profile that gives.
 */
export async function stubDeploymentProfile(
  page: Page,
  profile: {
    displayName: string | null;
    icon: { id: string; mimeType: string; downloadUrl: string } | null;
  },
): Promise<void> {
  await page.route("**/api/v1/deployment", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(profile) }),
  );
}

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
  await page.routeWebSocket(/\/api\/v1\/events(\?.*)?$/, (ws) => {
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
