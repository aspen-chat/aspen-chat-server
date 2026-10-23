import { generateKeyPairSync } from "node:crypto";
import { expect, test, type Page, type Route } from "@playwright/test";
import { signedIn, stubAuthMethods, stubSignedInBackend, uuid } from "./stubs";
import { signInToWorld } from "./world";

/**
 * Passkeys in the web client, under the relying party's domain (`localhost`, where the dev
 * server runs): autofill on the sign-in form, and the signals that keep the browser's passkey
 * managers in step with the server. Chromium alone runs them, with a virtual authenticator
 * through the DevTools protocol, which the other browsers lack.
 */
test.skip(({ browserName }) => browserName !== "chromium", "virtual authenticators are Chromium's");

const CREDENTIAL_ID = Buffer.from("aspen-e2e-credential").toString("base64url");

/** A server offering passkeys at `localhost`. Registered after `stubAuthMethods`, it wins. */
async function offerPasskeys(page: Page) {
  await page.route("**/api/v1/auth/methods", (route) =>
    route.fulfill({
      json: {
        passkeys: { rpId: "localhost", androidApps: [] },
        twoFactorRequired: false,
        registrationInviteRequired: false,
        protocol: { version: 1, minimum: 1 },
        software: { name: "aspen", version: "0.1.0" },
      },
    }),
  );
}

/** Answers every ceremony start with a `signIn` ceremony, counting them. */
async function stubCeremonies(page: Page, completion: (route: Route) => Promise<void>) {
  const starts: unknown[] = [];
  await page.route("**/api/v1/auth/passkey-ceremonies", (route) => {
    starts.push(route.request().postDataJSON());
    return route.fulfill({
      status: 201,
      json: {
        id: `cer-${String(starts.length)}`,
        purpose: "signIn",
        options: {
          publicKey: {
            challenge: Buffer.from(`challenge-${String(starts.length)}`).toString("base64url"),
            rpId: "localhost",
            allowCredentials: [],
            userVerification: "required",
            timeout: 300_000,
          },
        },
        expiresAt: new Date(Date.now() + 300_000).toISOString(),
      },
    });
  });
  await page.route("**/api/v1/auth/passkey-ceremonies/*/credential", completion);
  return starts;
}

/**
 * A platform authenticator holding one passkey of the account `uuid`, which it hands over
 * unasked; or, `empty`, one holding none, which leaves a conditional request waiting as a
 * browser with no passkeys for the site does (headless Chromium with no authenticator refuses
 * one at once).
 */
async function virtualAuthenticator(page: Page, { empty = false }: { empty?: boolean } = {}) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: !empty,
    },
  });
  if (empty) {
    return;
  }
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  await cdp.send("WebAuthn.addCredential", {
    authenticatorId,
    credential: {
      credentialId: Buffer.from("aspen-e2e-credential").toString("base64"),
      isResidentCredential: true,
      rpId: "localhost",
      privateKey: privateKey.export({ format: "der", type: "pkcs8" }).toString("base64"),
      userHandle: Buffer.from(uuid.replaceAll("-", ""), "hex").toString("base64"),
      signCount: 0,
    },
  });
}

/**
 * Records every signal the page sends and every conditional request it makes, in place of the
 * browser's own, which talk to passkey managers the test cannot see.
 */
async function recordPasskeyCalls(page: Page) {
  await page.addInitScript(() => {
    const calls: unknown[] = [];
    (window as unknown as { passkeyCalls: unknown[] }).passkeyCalls = calls;
    const record = (name: string) => (options: unknown) => {
      calls.push({ name, options });
      return Promise.resolve();
    };
    Object.assign(PublicKeyCredential, {
      signalUnknownCredential: record("signalUnknownCredential"),
      signalAllAcceptedCredentials: record("signalAllAcceptedCredentials"),
      signalCurrentUserDetails: record("signalCurrentUserDetails"),
    });
    const get = navigator.credentials.get.bind(navigator.credentials);
    navigator.credentials.get = (options?: CredentialRequestOptions) => {
      if (options?.mediation === "conditional") {
        calls.push({ name: "conditional" });
        options.signal?.addEventListener("abort", () => calls.push({ name: "aborted" }));
      }
      const answer = get(options);
      if (options?.mediation === "conditional") {
        answer.then(
          () => calls.push({ name: "resolved" }),
          (e: unknown) => calls.push({ name: "rejected", options: String(e) }),
        );
      }
      return answer;
    };
  });
}

async function passkeyCalls(page: Page): Promise<{ name: string; options?: unknown }[]> {
  return page.evaluate(
    () =>
      (window as unknown as { passkeyCalls: { name: string; options?: unknown }[] }).passkeyCalls,
  );
}

test("the sign-in form offers passkeys as the username's autofill, and stops for a password", async ({
  page,
}) => {
  await stubAuthMethods(page);
  await offerPasskeys(page);
  await recordPasskeyCalls(page);
  await virtualAuthenticator(page, { empty: true });
  const completions: unknown[] = [];
  const starts = await stubCeremonies(page, (route) => {
    completions.push(route.request().postDataJSON());
    return route.fulfill({ status: 500, json: { code: "internal", title: "no", status: 500 } });
  });
  await page.route("**/api/v1/auth/login", (route) =>
    route.fulfill({
      status: 401,
      contentType: "application/problem+json",
      body: JSON.stringify({ code: "invalidCredentials", title: "Wrong password", status: 401 }),
    }),
  );
  await page.goto("/");
  const username = page.getByLabel("Username");
  await expect(username).toHaveAttribute("autocomplete", "username webauthn");
  await expect
    .poll(async () => (await passkeyCalls(page)).map((c) => c.name))
    .toEqual(["conditional"]);
  expect(starts).toEqual([{ purpose: "signIn" }]);

  // Signing in with a password ends the waiting request first.
  await username.fill("kate");
  await page.getByLabel("Password").fill("wrong");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByText("Wrong password")).toBeVisible();
  expect((await passkeyCalls(page)).map((c) => c.name)).toEqual([
    "conditional",
    "aborted",
    "rejected",
  ]);

  // Returning to the username field offers them again, with a new ceremony; another focus
  // while it waits starts none.
  await page.getByLabel("Password").focus();
  await username.focus();
  await expect.poll(() => starts.length).toBe(2);
  await page.getByLabel("Password").focus();
  await username.focus();
  await page.waitForTimeout(300);
  expect(starts).toHaveLength(2);
  expect(completions).toEqual([]);
});

test("a passkey picked from autofill signs in", async ({ page }) => {
  await stubAuthMethods(page);
  await offerPasskeys(page);
  await stubSignedInBackend(page, "kate");
  await virtualAuthenticator(page);
  const completions: { credential: { id: string } }[] = [];
  await stubCeremonies(page, (route) => {
    completions.push(route.request().postDataJSON() as { credential: { id: string } });
    return route.fulfill({
      json: { outcome: "signedIn", ...(JSON.parse(signedIn(null)) as Record<string, unknown>) },
    });
  });
  await page.goto("/");
  await page.getByLabel("Username").focus();
  await expect.poll(() => completions.length, { timeout: 15_000 }).toBe(1);
  expect(completions[0]?.credential.id).toBe(CREDENTIAL_ID);
  await expect(page.getByLabel("Username")).toHaveCount(0);
});

test("a passkey the server no longer holds is named to the browser", async ({ page }) => {
  await stubAuthMethods(page);
  await offerPasskeys(page);
  await recordPasskeyCalls(page);
  await virtualAuthenticator(page);
  await stubCeremonies(page, (route) =>
    route.fulfill({
      status: 400,
      contentType: "application/problem+json",
      body: JSON.stringify({
        code: "passkeyUnknown",
        title: "This passkey isn't registered here.",
        detail: "This server has no account with that passkey.",
        status: 400,
      }),
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Sign in with a passkey" }).click();
  await expect(page.getByText("This server has no account with that passkey.")).toBeVisible();
  const unknown = (await passkeyCalls(page)).filter((c) => c.name === "signalUnknownCredential");
  expect(unknown.map((c) => c.options)).toContainEqual({
    rpId: "localhost",
    credentialId: CREDENTIAL_ID,
  });
});

test("the security settings name every passkey the account holds to the browser", async ({
  page,
}) => {
  await recordPasskeyCalls(page);
  await signInToWorld(page, async (p) => {
    await offerPasskeys(p);
    await p.route(/\/api\/v1\/users\/(%40|@)me\/security$/, (route) =>
      route.fulfill({
        json: {
          passkeys: [
            {
              id: uuid,
              credentialId: CREDENTIAL_ID,
              name: "Laptop",
              createdAt: new Date().toISOString(),
              lastUsedAt: null,
            },
          ],
          passkeyUserHandle: "AZDwoAAAcACAAAAAAAAAAQ",
          recoveryCodesRemaining: 10,
          totp: false,
          twoFactorEnabled: true,
          twoFactorRequired: false,
          verifiedUntil: new Date(Date.now() + 600_000).toISOString(),
        },
      }),
    );
    await p.route("**/api/v1/auth/passkey-ceremonies", (route) =>
      route.fulfill({ status: 429, json: { code: "rateLimited", title: "slow", status: 429 } }),
    );
  });
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Sign-in and security" }).click();
  await expect(page.getByText("Laptop")).toBeVisible();
  await expect
    .poll(async () =>
      (await passkeyCalls(page)).filter((c) => c.name === "signalAllAcceptedCredentials"),
    )
    .toContainEqual({
      name: "signalAllAcceptedCredentials",
      options: {
        rpId: "localhost",
        userId: "AZDwoAAAcACAAAAAAAAAAQ",
        allAcceptedCredentialIds: [CREDENTIAL_ID],
      },
    });
});
