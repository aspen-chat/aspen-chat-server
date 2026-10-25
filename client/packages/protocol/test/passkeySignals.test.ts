import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ApiProblemError,
  AspenClient,
  MemorySessionStore,
  PasskeyCancelledError,
  PasskeyUnavailableError,
  conditionalMediationAvailable,
  passkeySignalAvailable,
  signalAllAcceptedCredentials,
  signalUnknownCredential,
  type NativePasskeys,
} from "../src";

const baseUrl = "https://aspen.test";
const uuid = "0190f0a0-0000-7000-8000-000000000001";

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": status >= 400 ? "application/problem+json" : "application/json",
    },
  });
}

const signedIn = () =>
  json(200, {
    outcome: "signedIn",
    userId: uuid,
    refreshToken: "r",
    sessionToken: "s",
    sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
    twoFactorEnrollmentRequired: false,
  });

/** A fetch answering each request with the next responder, recording the paths asked. */
function scripted(responders: (() => Response)[]) {
  const paths: string[] = [];
  const fetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const request = new Request(input, init);
    paths.push(`${request.method} ${new URL(request.url).pathname}`);
    const next = responders.shift();
    if (next === undefined) {
      throw new Error(`unexpected ${request.url}`);
    }
    return Promise.resolve(next());
  };
  return { fetch, paths };
}

function started(id: string, expiresInMs = 300_000, rpId = "aspen.test") {
  return () =>
    json(201, {
      id,
      purpose: "signIn",
      options: { publicKey: { challenge: "AAEC", rpId, allowCredentials: [] } },
      expiresAt: new Date(Date.now() + expiresInMs).toISOString(),
    });
}

/** A browser credential as `navigator.credentials.get` returns one. */
function assertion(id: string) {
  const bytes = new Uint8Array([1, 2, 3]).buffer;
  return {
    id,
    rawId: bytes,
    type: "public-key",
    getClientExtensionResults: () => ({}),
    response: {
      clientDataJSON: bytes,
      authenticatorData: bytes,
      signature: bytes,
      userHandle: bytes,
    },
  };
}

/** Stands in for the browser's `PublicKeyCredential`, recording every signal sent. */
function stubSignals() {
  const sent: { name: string; options: unknown }[] = [];
  const record = (name: string) => (options: unknown) => {
    sent.push({ name, options });
    return Promise.resolve();
  };
  vi.stubGlobal("PublicKeyCredential", {
    isConditionalMediationAvailable: () => Promise.resolve(true),
    signalUnknownCredential: record("signalUnknownCredential"),
    signalAllAcceptedCredentials: record("signalAllAcceptedCredentials"),
    signalCurrentUserDetails: record("signalCurrentUserDetails"),
  });
  return sent;
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("passkey signals", () => {
  it("are sent where the browser has them, and dropped quietly where it does not", async () => {
    expect(passkeySignalAvailable("signalUnknownCredential")).toBe(false);
    signalUnknownCredential("aspen.test", "cred");
    vi.stubGlobal("PublicKeyCredential", {
      signalAllAcceptedCredentials: () => Promise.reject(new Error("refused")),
      signalUnknownCredential: () => {
        throw new Error("refused at once");
      },
    });
    expect(passkeySignalAvailable("signalAllAcceptedCredentials")).toBe(true);
    expect(passkeySignalAvailable("signalCurrentUserDetails")).toBe(false);
    signalAllAcceptedCredentials("aspen.test", "user", []);
    signalUnknownCredential("aspen.test", "cred");
    await Promise.resolve();
    const sent = stubSignals();
    signalAllAcceptedCredentials("aspen.test", "user", ["a", "b"]);
    expect(sent).toEqual([
      {
        name: "signalAllAcceptedCredentials",
        options: { rpId: "aspen.test", userId: "user", allAcceptedCredentialIds: ["a", "b"] },
      },
    ]);
  });

  it("names a passkey the server does not hold to the browser", async () => {
    const sent = stubSignals();
    vi.stubGlobal("navigator", {
      credentials: { get: () => Promise.resolve(assertion("gone")) },
    });
    const { fetch } = scripted([
      started("cer-1"),
      () => json(400, { code: "passkeyUnknown", title: "unknown", status: 400 }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(
      client.runPasskeyCeremony({ purpose: "signIn" }, { kind: "inPage", rpId: "aspen.test" }),
    ).rejects.toBeInstanceOf(ApiProblemError);
    expect(sent).toEqual([
      { name: "signalUnknownCredential", options: { rpId: "aspen.test", credentialId: "gone" } },
    ]);
  });
});

describe("conditional passkey sign-in", () => {
  it("is offered where the browser says so", async () => {
    expect(await conditionalMediationAvailable()).toBe(false);
    stubSignals();
    expect(await conditionalMediationAvailable()).toBe(true);
  });

  it("asks for autofill and signs in with the passkey picked", async () => {
    let asked: CredentialRequestOptions | undefined;
    vi.stubGlobal("navigator", {
      credentials: {
        get: (options: CredentialRequestOptions) => {
          asked = options;
          return Promise.resolve(assertion("picked"));
        },
      },
    });
    const store = new MemorySessionStore();
    const { fetch, paths } = scripted([started("cer-2"), signedIn]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const outcome = await client.runConditionalPasskeySignIn(
      "aspen.test",
      new AbortController().signal,
    );
    expect(outcome.outcome).toBe("signedIn");
    expect(asked?.mediation).toBe("conditional");
    expect(store.load()?.sessionToken).toBe("s");
    expect(paths).toEqual([
      "POST /api/v1/auth/passkey-ceremonies",
      "POST /api/v1/auth/passkey-ceremonies/cer-2/credential",
    ]);
  });

  /** A browser whose conditional request waits until it is aborted, as one does until a pick. */
  function waitingBrowser() {
    vi.stubGlobal("navigator", {
      credentials: {
        get: ({ signal }: CredentialRequestOptions) =>
          new Promise((_, reject) => {
            signal?.addEventListener("abort", () => {
              reject(new DOMException("aborted", "AbortError"));
            });
          }),
      },
    });
  }

  it("is unavailable, not failed, when the server will not start one", async () => {
    const { fetch } = scripted([
      () => json(429, { code: "rateLimited", title: "slow down", status: 429 }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(
      client.runConditionalPasskeySignIn("aspen.test", new AbortController().signal),
    ).rejects.toBeInstanceOf(PasskeyUnavailableError);
  });

  it("ends as cancelled when the form signs in another way", async () => {
    waitingBrowser();
    const { fetch, paths } = scripted([started("cer-3")]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const controller = new AbortController();
    const waiting = client.runConditionalPasskeySignIn("aspen.test", controller.signal);
    await vi.waitFor(() => {
      expect(paths).toHaveLength(1);
    });
    controller.abort();
    await expect(waiting).rejects.toBeInstanceOf(PasskeyCancelledError);
    expect(paths).toHaveLength(1);
  });

  it("stops offering passkeys before its ceremony expires", async () => {
    waitingBrowser();
    const { fetch, paths } = scripted([started("cer-4", 15_050)]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(
      client.runConditionalPasskeySignIn("aspen.test", new AbortController().signal),
    ).rejects.toBeInstanceOf(PasskeyCancelledError);
    expect(paths).toHaveLength(1);
  });
});

describe("native passkeys", () => {
  const handoff = (opened: string[]) => ({
    prepare: () =>
      Promise.resolve({
        returnTo: "aspen://auth/passkey",
        open: (url: string) => {
          opened.push(url);
          return Promise.resolve({ ceremony: "cer-6", outcome: "done" as const, code: "c" });
        },
        dispose: () => undefined,
      }),
  });

  it("run in the app and complete as this sign-in", async () => {
    const asked: unknown[] = [];
    const native: NativePasskeys = {
      create: () => Promise.reject(new Error("not asked")),
      get: (publicKey, host) => {
        asked.push(publicKey, host);
        return Promise.resolve({ id: "native" });
      },
    };
    const { fetch, paths } = scripted([started("cer-5"), signedIn]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const opened: string[] = [];
    const outcome = await client.runPasskeyCeremony(
      { purpose: "signIn" },
      { kind: "native", native, fallback: handoff(opened) },
    );
    expect(outcome.outcome).toBe("signedIn");
    expect(asked).toEqual([
      { challenge: "AAEC", rpId: "aspen.test", allowCredentials: [] },
      "aspen.test",
    ]);
    expect(opened).toEqual([]);
    expect(paths).toEqual([
      "POST /api/v1/auth/passkey-ceremonies",
      "POST /api/v1/auth/passkey-ceremonies/cer-5/credential",
    ]);
  });

  it("go to the system browser when the platform cannot run them", async () => {
    const native: NativePasskeys = {
      create: () => Promise.reject(new PasskeyUnavailableError("no provider")),
      get: () => Promise.reject(new PasskeyUnavailableError("no provider")),
    };
    const { fetch, paths } = scripted([started("cer-5"), started("cer-6"), signedIn]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const opened: string[] = [];
    const outcome = await client.runPasskeyCeremony(
      { purpose: "signIn" },
      { kind: "native", native, fallback: handoff(opened) },
    );
    expect(outcome.outcome).toBe("signedIn");
    expect(opened).toEqual([`${baseUrl}/auth/passkey#ceremony=cer-6`]);
    expect(paths).toEqual([
      "POST /api/v1/auth/passkey-ceremonies",
      "POST /api/v1/auth/passkey-ceremonies",
      "POST /api/v1/auth/passkey-ceremonies/cer-6/claim",
    ]);
  });

  it("go to the system browser when the options name another deployment's relying party", async () => {
    const asked: unknown[] = [];
    const native: NativePasskeys = {
      create: (publicKey) => {
        asked.push(publicKey);
        return Promise.resolve({ id: "native" });
      },
      get: (publicKey) => {
        asked.push(publicKey);
        return Promise.resolve({ id: "native" });
      },
    };
    const { fetch, paths } = scripted([
      started("cer-5", 300_000, "victim.example"),
      started("cer-6"),
      signedIn,
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const opened: string[] = [];
    const outcome = await client.runPasskeyCeremony(
      { purpose: "signIn" },
      { kind: "native", native, fallback: handoff(opened) },
    );
    expect(outcome.outcome).toBe("signedIn");
    expect(asked).toEqual([]);
    expect(opened).toEqual([`${baseUrl}/auth/passkey#ceremony=cer-6`]);
    expect(paths).toEqual([
      "POST /api/v1/auth/passkey-ceremonies",
      "POST /api/v1/auth/passkey-ceremonies",
      "POST /api/v1/auth/passkey-ceremonies/cer-6/claim",
    ]);
  });

  it("end where the person dismissed the prompt", async () => {
    const native: NativePasskeys = {
      create: () => Promise.reject(new PasskeyCancelledError()),
      get: () => Promise.reject(new PasskeyCancelledError()),
    };
    const { fetch, paths } = scripted([started("cer-7")]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(
      client.runPasskeyCeremony(
        { purpose: "signIn" },
        { kind: "native", native, fallback: handoff([]) },
      ),
    ).rejects.toBeInstanceOf(PasskeyCancelledError);
    expect(paths).toHaveLength(1);
  });
});
