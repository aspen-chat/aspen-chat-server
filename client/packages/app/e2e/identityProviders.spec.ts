import AxeBuilder from "@axe-core/playwright";
import { createHash } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { signedIn, stubAuthMethods, stubSignedInBackend } from "./stubs";
import { settleAnimations, signInToWorld } from "./world";

/**
 * Signing in through an identity provider, linking and unlinking one, and the dashboard's
 * providers, against stubbed servers. The provider is a page at `idp.example.test` that sends
 * the browser straight back to the web client's return page, as the server does once the
 * provider has answered.
 */

const provider = { id: "0190f0a0-0000-7000-8000-0000000000aa", name: "Example Corp" };
const flowId = "flow-1";
const returnCode = "return-code-1";

function problem(status: number, code: string, detail: string) {
  return {
    status,
    contentType: "application/problem+json",
    body: JSON.stringify({ code, title: code, detail, status }),
  };
}

/** What axe finds wrong with the page as it stands, in the default appearance. */
async function axeProblems(page: Page): Promise<string[]> {
  await settleAnimations(page);
  const audit = await new AxeBuilder({ page }).analyze();
  return audit.violations.flatMap((violation) =>
    violation.nodes.map((node) => `${violation.id} at ${node.target.join(" ")}`),
  );
}

/** `BASE64URL(SHA256(verifier))`, the `S256` challenge a verifier answers. */
function challengeOf(verifier: string): string {
  return createHash("sha256").update(verifier).digest("base64url");
}

/**
 * A signed-out server offering Example Corp, whose flows return to this page at once. Answers
 * what the claim does with `claim`; records each flow started and each claim.
 */
async function stubProvider(
  page: Page,
  claim: () => { status: number; contentType: string; body: string },
) {
  await stubAuthMethods(page);
  return stubFlows(page, claim);
}

/** What a spec's flows were started and claimed with. */
interface FlowRecord {
  started: Record<string, unknown>[];
  claimed: Record<string, unknown>[];
}

/**
 * Example Corp's flows, offered by `GET /auth/methods` and returning to this page at once, noted
 * in `record`.
 */
async function stubFlows(
  page: Page,
  claim: () => { status: number; contentType: string; body: string },
  record: FlowRecord = { started: [], claimed: [] },
): Promise<FlowRecord> {
  const { started, claimed } = record;
  await page.route("**/api/v1/auth/methods", (route) =>
    route.fulfill({
      json: {
        passkeys: null,
        twoFactorRequired: false,
        registrationInviteRequired: false,
        protocol: { version: 1, minimum: 1, capabilities: [] },
        software: { name: "aspen", version: "0.1.0" },
        identityProviders: [provider],
      },
    }),
  );
  await page.route("**/api/v1/auth/identity-provider-flows", (route) => {
    started.push(route.request().postDataJSON() as Record<string, unknown>);
    return route.fulfill({
      status: 201,
      json: {
        id: flowId,
        authorizeUrl: `https://idp.example.test/authorize?state=${flowId}`,
        expiresAt: new Date(Date.now() + 600_000).toISOString(),
      },
    });
  });
  await page.route("https://idp.example.test/**", (route) => {
    const back = `${new URL(page.url()).origin}/sign-in/provider#flow=${flowId}&code=${returnCode}`;
    return route.fulfill({
      contentType: "text/html",
      body: `<!doctype html><script>location.replace(${JSON.stringify(back)})</script>`,
    });
  });
  await page.route(`**/api/v1/auth/identity-provider-flows/${flowId}/claim`, (route) => {
    claimed.push(route.request().postDataJSON() as Record<string, unknown>);
    return route.fulfill(claim());
  });
  return record;
}

test.describe("signing in through an identity provider", () => {
  test("signs in to the linked account, claiming with the verifier it started with", async ({
    page,
  }) => {
    const { started, claimed } = await stubProvider(page, () => ({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        outcome: "signedIn",
        ...(JSON.parse(signedIn()) as Record<string, unknown>),
      }),
    }));
    await stubSignedInBackend(page, "kate");
    await page.goto("/");
    await page.getByRole("button", { name: "Continue with Example Corp" }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    // Back where the sign-in started, with the spent return code gone from the address.
    expect(new URL(page.url()).pathname).toBe("/");
    expect(new URL(page.url()).hash).toBe("");
    expect(started).toEqual([
      { provider: provider.id, purpose: "signIn", codeChallenge: expect.any(String) as unknown },
    ]);
    expect(claimed).toHaveLength(1);
    expect(claimed[0]?.code).toBe(returnCode);
    expect(challengeOf(String(claimed[0]?.codeVerifier))).toBe(started[0]?.codeChallenge);
  });

  test("makes a new account, trying another name when one is taken", async ({ page }) => {
    await stubProvider(page, () => ({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        outcome: "accountRequired",
        providerName: "Example Corp",
        suggestedName: "kate",
        displayName: "Kate Example",
        inviteRequired: true,
        email: "kate@example.org",
        emailRequired: false,
      }),
    }));
    const accounts: Record<string, unknown>[] = [];
    await page.route(`**/api/v1/auth/identity-provider-flows/${flowId}/account`, (route) => {
      accounts.push(route.request().postDataJSON() as Record<string, unknown>);
      return route.fulfill(
        accounts.length === 1
          ? problem(409, "usernameTaken", "That username is taken.")
          : { status: 201, contentType: "application/json", body: signedIn(null) },
      );
    });
    await stubSignedInBackend(page, "kate.example");
    await page.goto("/");
    await page.getByRole("button", { name: "Continue with Example Corp" }).click();

    await expect(page.getByRole("heading", { name: "Choose a username" })).toBeVisible();
    expect(await axeProblems(page)).toEqual([]);
    await expect(page.getByText("Your display name will be Kate Example.")).toBeVisible();
    await expect(
      page.getByText("Your email address will be kate@example.org, as Example Corp confirmed it."),
    ).toBeVisible();
    const username = page.getByLabel("Username");
    await expect(username).toHaveValue("kate");
    await page.getByLabel("Invite code").fill("WELCOME1");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByText("That username is taken.")).toBeVisible();
    await expect(username).toHaveAttribute("aria-invalid", "true");

    await username.fill("kate.example");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByRole("heading", { name: "No communities yet" })).toBeVisible();
    expect(accounts.map((a) => [a.name, a.inviteCode])).toEqual([
      ["kate", "WELCOME1"],
      ["kate.example", "WELCOME1"],
    ]);
    // The same verifier the claim presented makes the account.
    expect(new Set(accounts.map((a) => a.codeVerifier)).size).toBe(1);
  });

  test("shows why the provider did not sign them in, with a way back", async ({ page }) => {
    await stubProvider(page, () =>
      problem(401, "identityProviderFailed", "Example Corp declined to sign you in."),
    );
    await page.goto("/");
    await page.getByRole("button", { name: "Continue with Example Corp" }).click();
    await expect(page.getByRole("heading", { name: "Couldn't sign in" })).toBeVisible();
    await expect(page.getByRole("alert")).toHaveText("Example Corp declined to sign you in.");
    expect(await axeProblems(page)).toEqual([]);
    await page.getByRole("button", { name: "Sign in another way" }).click();
    await expect(page.getByRole("heading", { name: "Sign in" })).toBeVisible();
  });

  test("says a return this tab did not start cannot be finished", async ({ page }) => {
    await stubProvider(page, () => problem(500, "internal", "unreachable"));
    await page.goto(`/sign-in/provider#flow=someone-elses&code=${returnCode}`);
    await expect(page.getByRole("alert")).toHaveText(
      "This sign-in was not started in this tab, or it was started too long ago. Start again from the sign-in screen.",
    );
  });
});

/** Opens Sign-in and security, and finds the section named `name` in it. */
async function securitySection(page: Page, name: string) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Sign-in and security" }).click();
  return page
    .getByRole("dialog", { name: "Sign-in and security" })
    .getByRole("region", { name, exact: true });
}

/** Security settings of an account with a password and the links `identities`. */
function securitySettings(identities: unknown[], { hasPassword = true, verified = true } = {}) {
  return {
    hasPassword,
    identities,
    passkeys: [],
    recoveryCodesRemaining: 0,
    totp: false,
    twoFactorEnabled: false,
    twoFactorRequired: false,
    verifiedUntil: verified ? new Date(Date.now() + 600_000).toISOString() : null,
  };
}

const linkedIdentity = {
  id: "identity-1",
  provider: provider.id,
  providerName: "Example Corp",
  createdAt: "2026-10-01T12:00:00Z",
  lastUsedAt: "2026-10-20T12:00:00Z",
};

test.describe("an account made through a provider", () => {
  const passwords: Record<string, unknown>[] = [];

  test.beforeEach(async ({ page }) => {
    passwords.length = 0;
    await signInToWorld(page, async (p) => {
      await p.route(/\/api\/v1\/users\/(%40|@)me\/security$/, (route) =>
        route.fulfill({ json: securitySettings([linkedIdentity], { hasPassword: false }) }),
      );
      await p.route(/\/api\/v1\/users\/(%40|@)me\/identities\/identity-1$/, (route) =>
        route.fulfill(
          problem(
            409,
            "lastSignInMethod",
            "Example Corp is your only way to sign in. Add a password or a passkey first.",
          ),
        ),
      );
      await p.route(/\/api\/v1\/users\/(%40|@)me\/password$/, (route) => {
        passwords.push(route.request().postDataJSON() as Record<string, unknown>);
        return route.fulfill({ status: 204 });
      });
    });
  });

  test("is refused unlinking its only way in, and says why", async ({ page }) => {
    const section = await securitySection(page, "Identity providers");
    await expect(section.getByText("Example Corp", { exact: true })).toBeVisible();
    await section.getByRole("button", { name: "Unlink Example Corp" }).click();
    await expect(
      page
        .getByRole("dialog", { name: "Sign-in and security" })
        .getByRole("alert")
        .filter({ hasText: "Example Corp is your only way to sign in." }),
    ).toBeVisible();
  });

  test("sets its first password without a current one", async ({ page }) => {
    const section = await securitySection(page, "Password");
    await expect(section.getByText(/Your account has no password/)).toBeVisible();
    await section.getByRole("button", { name: "Set a password" }).click();
    await expect(section.getByLabel("Current password")).toHaveCount(0);
    await section.getByLabel("New password", { exact: true }).fill("a new password");
    await section.getByLabel("Confirm new password").fill("a new password");
    await section.getByRole("button", { name: "Set a password" }).click();
    await expect(
      section.getByText("Password set. You've been signed out everywhere else."),
    ).toBeVisible();
    expect(passwords).toEqual([{ newPassword: "a new password" }]);
  });
});

test.describe("going to a provider from the web client signed in", () => {
  test("links one, and says so back where the tab was", async ({ page }) => {
    const flows: FlowRecord = { started: [], claimed: [] };
    await signInToWorld(page, async (p) => {
      await stubFlows(
        p,
        () => ({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ outcome: "linked", identity: linkedIdentity }),
        }),
        flows,
      );
      await p.route(/\/api\/v1\/users\/(%40|@)me\/security$/, (route) =>
        route.fulfill({ json: securitySettings([]) }),
      );
    });
    const section = await securitySection(page, "Identity providers");
    await expect(section.getByText("None linked.")).toBeVisible();
    const before = new URL(page.url()).pathname;
    await section.getByRole("button", { name: "Link Example Corp" }).click();
    await expect(page.getByText("Example Corp is linked to your account.")).toBeVisible();
    expect(new URL(page.url()).pathname).toBe(before);
    expect(flows.started.map((start) => start.purpose)).toEqual(["link"]);
  });

  test("verifies again through a linked provider, the only way an account without a password has", async ({
    page,
  }) => {
    const flows: FlowRecord = { started: [], claimed: [] };
    await signInToWorld(page, async (p) => {
      await stubFlows(
        p,
        () => ({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            outcome: "reauthenticated",
            verifiedUntil: new Date(Date.now() + 600_000).toISOString(),
          }),
        }),
        flows,
      );
      await p.route(/\/api\/v1\/users\/(%40|@)me\/security$/, (route) =>
        route.fulfill({
          json: securitySettings([linkedIdentity], { hasPassword: false, verified: false }),
        }),
      );
      await p.route(/\/api\/v1\/users\/(%40|@)me\/identities\/identity-1$/, (route) =>
        route.fulfill(problem(403, "reauthenticationRequired", "Confirm it's you first.")),
      );
    });
    const section = await securitySection(page, "Identity providers");
    await section.getByRole("button", { name: "Unlink Example Corp" }).click();
    const confirm = page.getByRole("dialog", { name: "Confirm it's you" });
    await expect(confirm.getByText(/with a passkey or an identity provider/)).toBeVisible();
    await expect(confirm.getByLabel("Password")).toHaveCount(0);
    await expect(confirm.getByRole("button", { name: "Confirm", exact: true })).toHaveCount(0);
    expect(await axeProblems(page)).toEqual([]);
    await confirm.getByRole("button", { name: "Confirm with Example Corp" }).click();
    await expect(
      page.getByText("Confirmed with Example Corp. Make your change again."),
    ).toBeVisible();
    expect(flows.started.map((start) => start.purpose)).toEqual(["reauthenticate"]);
  });
});

test.describe("the dashboard's identity providers", () => {
  test("adds a provider once it can be reached, and removes one, naming its links", async ({
    page,
  }) => {
    const providers = [
      {
        id: "0190f0a0-0000-7000-8000-0000000000bb",
        name: "Big Corp",
        issuer: "https://login.big.example/realms/staff",
        clientId: "aspen",
        enabled: true,
        skipInvite: false,
        enforcesTwoFactor: true,
        trustEmail: true,
        createdAt: "2026-09-01T12:00:00Z",
        linkedAccounts: 1001,
      },
    ];
    const added: Record<string, unknown>[] = [];
    await signInToWorld(page, async (p) => {
      await p.route(/\/api\/v1\/users\/(%40|@)me\/admin$/, (route) =>
        route.fulfill({
          json: {
            permissions: ["viewDashboard", "manageIdentityProviders"],
            roles: [],
            inclusions: [],
          },
        }),
      );
      await p.route(/\/api\/v1\/admin\/identity-providers$/, (route) => {
        if (route.request().method() === "GET") {
          return route.fulfill({
            json: {
              providers,
              redirectUri: "https://chat.example.org/auth/oidc/callback",
              backChannelLogoutUri: "https://chat.example.org/auth/oidc/back-channel-logout",
              maxProviders: 20,
            },
          });
        }
        const body = route.request().postDataJSON() as Record<string, unknown>;
        added.push(body);
        if (added.length === 1) {
          return route.fulfill(
            problem(
              502,
              "identityProviderUnreachable",
              "idp.example.test could not be reached. Check the issuer URL.",
            ),
          );
        }
        const made = {
          id: "0190f0a0-0000-7000-8000-0000000000cc",
          name: String(body.name),
          issuer: String(body.issuer),
          clientId: String(body.clientId),
          enabled: true,
          skipInvite: false,
          enforcesTwoFactor: false,
          trustEmail: true,
          createdAt: new Date().toISOString(),
          linkedAccounts: 0,
        };
        providers.push(made);
        return route.fulfill({ status: 201, json: made });
      });
      await p.route(/\/api\/v1\/admin\/identity-providers\/[^/]+$/, (route) => {
        const id = new URL(route.request().url()).pathname.split("/").pop();
        providers.splice(
          providers.findIndex((provider) => provider.id === id),
          1,
        );
        return route.fulfill({ status: 204 });
      });
    });

    await page
      .getByRole("navigation", { name: "Communities" })
      .getByRole("link", { name: "Administration" })
      .click();
    await page
      .getByRole("navigation", { name: "Administration sections" })
      .getByRole("link", { name: "Identity providers", exact: true })
      .click();
    const section = page.getByRole("region", { name: "Identity providers" });
    await expect(section.getByText("Big Corp")).toBeVisible();
    await expect(section.getByText("1,000+ linked accounts")).toBeVisible();
    const addresses = page.getByRole("region", { name: "Registering this server" });
    await expect(addresses.getByText("https://chat.example.org/auth/oidc/callback")).toBeVisible();
    await expect(addresses.getByRole("button", { name: "Copy Redirect URI" })).toBeVisible();

    const form = section.getByRole("form", { name: "Add an identity provider" });
    await form.getByLabel("Name").fill("Example Corp");
    await form.getByLabel("Issuer URL").fill("https://idp.example.test");
    await form.getByLabel("Client ID").fill("aspen-chat");
    await form.getByLabel("Client secret").fill("s3cret");
    await form.getByRole("button", { name: "Add provider" }).click();
    await expect(form.getByRole("alert")).toHaveText(
      "idp.example.test could not be reached. Check the issuer URL.",
    );
    await form.getByRole("button", { name: "Add provider" }).click();
    await expect(section.getByText("Example Corp")).toBeVisible();
    await expect(form.getByLabel("Name")).toHaveValue("");
    expect(added.at(-1)).toEqual({
      name: "Example Corp",
      issuer: "https://idp.example.test",
      clientId: "aspen-chat",
      clientSecret: "s3cret",
      enabled: true,
      skipInvite: false,
      enforcesTwoFactor: false,
      trustEmail: false,
    });

    await section.getByRole("button", { name: "Remove Big Corp" }).click();
    const confirm = page.getByRole("alertdialog", { name: "Remove Big Corp?" });
    await expect(confirm.getByText(/^1,000\+ accounts are linked to it\./)).toBeVisible();
    await expect(confirm.getByText(/will no longer be able to sign in/)).toBeVisible();
    await confirm.getByRole("button", { name: "Remove" }).click();
    await expect(section.getByText("Big Corp")).toHaveCount(0);
  });
});
