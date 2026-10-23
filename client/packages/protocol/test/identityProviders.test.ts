import { describe, expect, it } from "vitest";
import {
  ApiProblemError,
  AspenClient,
  MemorySessionStore,
  PasskeyCancelledError,
  offeredProviders,
  parseHandoffReturn,
  parseProviderFragment,
  type AuthMethods,
  type HandoffReturn,
  type PasskeyHandoff,
} from "../src";

const baseUrl = "https://aspen.test";
const uuid = "0190f0a0-0000-7000-8000-000000000001";
const provider = "0190f0a0-0000-7000-8000-0000000000aa";

function json(status: number, body: unknown, problem = false): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": problem ? "application/problem+json" : "application/json" },
  });
}

interface Sent {
  url: string;
  method: string;
  body: Record<string, unknown> | null;
}

/** A scripted fetch: each call takes the next answer and records what it was sent. */
function scripted(answers: ((sent: Sent) => Response)[]) {
  const sent: Sent[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const request = new Request(input, init);
    const text = await request.text();
    const call: Sent = {
      url: request.url,
      method: request.method,
      body: text === "" ? null : (JSON.parse(text) as Record<string, unknown>),
    };
    sent.push(call);
    const answer = answers.shift();
    if (answer === undefined) {
      throw new Error(`unexpected request ${call.method} ${call.url}`);
    }
    return answer(call);
  };
  return { fetch, sent };
}

const loginResponse = {
  userId: uuid,
  refreshToken: "r",
  sessionToken: "s",
  sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
  twoFactorEnrollmentRequired: false,
};

const started = {
  id: "flow-1",
  authorizeUrl: "https://idp.example.org/authorize?state=flow-1",
  expiresAt: new Date(Date.now() + 600_000).toISOString(),
};

/** A hand-off whose browser comes back with `returned`, recording what it opened. */
function fakeHandoff(returned: HandoffReturn) {
  const opened: string[] = [];
  let disposed = 0;
  const handoff: PasskeyHandoff = {
    prepare: () =>
      Promise.resolve({
        returnTo: "http://127.0.0.1:41234/passkey",
        open: (url: string) => {
          opened.push(url);
          return Promise.resolve(returned);
        },
        dispose: () => {
          disposed += 1;
        },
      }),
  };
  return { handoff, opened, disposed: () => disposed };
}

describe("identity provider returns", () => {
  it("reads the web client's return fragment", () => {
    expect(parseProviderFragment("#flow=abc&code=xyz")).toEqual({ flow: "abc", code: "xyz" });
    expect(parseProviderFragment("flow=a%2Bb&code=c")).toEqual({ flow: "a+b", code: "c" });
    expect(parseProviderFragment("#flow=abc")).toBeNull();
    expect(parseProviderFragment("#code=xyz&flow=")).toBeNull();
    expect(parseProviderFragment("")).toBeNull();
  });

  it("reads an app's return as a finished hand-off named by the flow", () => {
    expect(parseHandoffReturn("http://127.0.0.1:5000/passkey?flow=abc&code=xyz")).toEqual({
      ceremony: "abc",
      outcome: "done",
      code: "xyz",
    });
    expect(parseHandoffReturn("aspen://auth/passkey?flow=abc&code=xyz")).toEqual({
      ceremony: "abc",
      outcome: "done",
      code: "xyz",
    });
    // A passkey ceremony's return is read as before, whatever else its query holds.
    expect(
      parseHandoffReturn("aspen://auth/passkey?ceremony=c&outcome=cancelled&flow=f&code=x"),
    ).toEqual({ ceremony: "c", outcome: "cancelled", code: "x" });
    expect(parseHandoffReturn("aspen://auth/passkey?flow=abc")).toBeNull();
  });

  it("offers no providers for a server that predates them", () => {
    const methods = { passkeys: null, registrationInviteRequired: false } as unknown as AuthMethods;
    expect(offeredProviders(methods)).toEqual([]);
    expect(
      offeredProviders({ ...methods, identityProviders: [{ id: provider, name: "Example" }] }),
    ).toEqual([{ id: provider, name: "Example" }]);
  });
});

describe("AspenClient's identity provider flows", () => {
  it("starts a flow with a challenge of the verifier it claims with, and signs in", async () => {
    const { fetch, sent } = scripted([
      () => json(201, started),
      () => json(200, { outcome: "signedIn", status: "signedIn", ...loginResponse }),
    ]);
    const store = new MemorySessionStore();
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const { flow, verifier } = await client.startIdentityProviderFlow(provider, "signIn");
    expect(flow).toEqual(started);
    expect(sent[0]?.url).toBe(`${baseUrl}/api/v1/auth/identity-provider-flows`);
    // The web client's flows return to its own page, so they name no return address.
    expect(sent[0]?.body).toEqual({
      provider,
      purpose: "signIn",
      codeChallenge: expect.stringMatching(/^[A-Za-z0-9_-]{43}$/) as unknown,
    });
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
    const challenge = btoa(String.fromCharCode(...new Uint8Array(digest)))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
    expect(sent[0]?.body?.codeChallenge).toBe(challenge);

    const outcome = await client.claimIdentityProviderFlow(flow.id, verifier, "return-code");
    expect(sent[1]?.url).toBe(`${baseUrl}/api/v1/auth/identity-provider-flows/flow-1/claim`);
    expect(sent[1]?.body).toEqual({ codeVerifier: verifier, code: "return-code" });
    expect(outcome.outcome).toBe("signedIn");
    expect(store.load()?.sessionToken).toBe("s");
  });

  it("leaves a second factor owed, storing no session", async () => {
    const { fetch } = scripted([
      () =>
        json(200, {
          outcome: "signedIn",
          status: "secondFactorRequired",
          ticket: "ticket-1",
          methods: ["totp", "recoveryCode"],
        }),
    ]);
    const store = new MemorySessionStore();
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    expect(await client.claimIdentityProviderFlow("flow-1", "v", "c")).toEqual({
      outcome: "secondFactorRequired",
      ticket: "ticket-1",
      methods: ["totp", "recoveryCode"],
    });
    expect(store.load()).toBeNull();
  });

  it("offers an account, then makes it with the same verifier", async () => {
    const offer = {
      providerName: "Example Corp",
      suggestedName: "kate",
      displayName: "Kate",
      inviteRequired: true,
      email: null,
      emailRequired: false,
    };
    const { fetch, sent } = scripted([
      () => json(200, { outcome: "accountRequired", ...offer }),
      () => json(409, { code: "usernameTaken", title: "Taken", status: 409 }, true),
      () => json(201, loginResponse),
    ]);
    const store = new MemorySessionStore();
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const outcome = await client.claimIdentityProviderFlow("flow-1", "verifier-1", "c");
    if (outcome.outcome !== "accountRequired") {
      throw new Error(`expected an offer, not ${outcome.outcome}`);
    }
    expect(outcome.offer).toMatchObject(offer);
    expect(outcome.account).toEqual({ flow: "flow-1", verifier: "verifier-1" });

    const taken = await client
      .createIdentityProviderAccount(outcome.account, { name: "kate", inviteCode: "ABC" })
      .catch((e: unknown) => e);
    expect(taken).toBeInstanceOf(ApiProblemError);
    expect((taken as ApiProblemError).code).toBe("usernameTaken");

    // A name typed decomposed is sent composed.
    await client.createIdentityProviderAccount(outcome.account, {
      name: "Zoë",
      inviteCode: "ABC",
    });
    expect(sent[2]?.url).toBe(`${baseUrl}/api/v1/auth/identity-provider-flows/flow-1/account`);
    expect(sent[2]?.body).toEqual({
      codeVerifier: "verifier-1",
      name: "Zoë",
      inviteCode: "ABC",
    });
    expect(store.load()?.sessionToken).toBe("s");
  });

  it("shows why a provider failed, as the server put it", async () => {
    const { fetch } = scripted([
      () =>
        json(
          401,
          {
            code: "identityProviderFailed",
            title: "Sign-in failed",
            detail: "Example Corp declined to sign you in.",
            status: 401,
          },
          true,
        ),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const failure = await client.claimIdentityProviderFlow("f", "v", "c").catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ApiProblemError);
    expect((failure as ApiProblemError).message).toBe("Example Corp declined to sign you in.");
  });

  it("runs a flow through an app's hand-off, returning to its address", async () => {
    const { fetch, sent } = scripted([
      () => json(201, started),
      () => json(200, { outcome: "linked", identity: { id: "i", provider } }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const { handoff, opened, disposed } = fakeHandoff({
      ceremony: "flow-1",
      outcome: "done",
      code: "back",
    });
    const outcome = await client.runIdentityProviderHandoff(provider, "link", handoff);
    expect(outcome).toEqual({ outcome: "linked", identity: { id: "i", provider } });
    expect(sent[0]?.body?.returnTo).toBe("http://127.0.0.1:41234/passkey");
    expect(opened).toEqual([started.authorizeUrl]);
    expect(sent[1]?.body?.code).toBe("back");
    expect(disposed()).toBe(1);
  });

  it("gives up on a hand-off whose browser was closed, claiming nothing", async () => {
    const { fetch, sent } = scripted([() => json(201, started)]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const { handoff, disposed } = fakeHandoff({ ceremony: "", outcome: "cancelled", code: null });
    await expect(client.runIdentityProviderHandoff(provider, "signIn", handoff)).rejects.toThrow(
      PasskeyCancelledError,
    );
    expect(sent).toHaveLength(1);
    expect(disposed()).toBe(1);
  });
});
