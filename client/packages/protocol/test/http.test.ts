import { describe, expect, it } from "vitest";
import {
  ApiProblemError,
  AspenClient,
  MemorySessionStore,
  PasskeyCancelledError,
  setPreferredLanguages,
  type Session,
} from "../src";

const baseUrl = "https://aspen.test";
const uuid = "0190f0a0-0000-7000-8000-000000000001";

function jsonResponse(status: number, body: unknown, problem = false): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": problem ? "application/problem+json" : "application/json" },
  });
}

function problem(status: number, code: string): Response {
  return jsonResponse(status, { code, title: code, status }, true);
}

interface Recorded {
  url: string;
  method: string;
  authorization: string | null;
  language: string | null;
  body: string;
}

/** A scripted fetch: each call pops the next responder and records what it was sent. */
function scriptedFetch(responders: ((r: Recorded) => Response)[]) {
  const calls: Recorded[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const request = new Request(input, init);
    const recorded: Recorded = {
      url: request.url,
      method: request.method,
      authorization: request.headers.get("authorization"),
      language: request.headers.get("accept-language"),
      body: await request.text(),
    };
    calls.push(recorded);
    const responder = responders.shift();
    if (responder === undefined) {
      throw new Error(`unexpected request ${recorded.method} ${recorded.url}`);
    }
    return responder(recorded);
  };
  return { fetch, calls };
}

function farFuture(): string {
  return new Date(Date.now() + 3 * 60 * 60 * 1000).toISOString();
}

function liveSession(): Session {
  return {
    userId: uuid,
    refreshToken: "refresh-1",
    sessionToken: "session-1",
    sessionTokenExpires: farFuture(),
  };
}

describe("AspenClient", () => {
  it("names the languages the user reads in every request", async () => {
    const { fetch, calls } = scriptedFetch([
      () => problem(401, "invalidCredentials"),
      () => problem(401, "invalidCredentials"),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await client.login("kate", "wrong").catch(() => undefined);
    setPreferredLanguages(["en-XA", "en"]);
    try {
      await client.login("kate", "wrong").catch(() => undefined);
    } finally {
      setPreferredLanguages([]);
    }
    expect(calls.map((call) => call.language)).toEqual([null, "en-XA, en"]);
  });

  it("logs in and stores the session", async () => {
    const store = new MemorySessionStore();
    const { fetch, calls } = scriptedFetch([
      () =>
        jsonResponse(200, {
          status: "signedIn",
          userId: uuid,
          refreshToken: "r",
          sessionToken: "s",
          sessionTokenExpires: farFuture(),
          twoFactorEnrollmentRequired: false,
        }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const outcome = await client.login("kate", "hunter22");
    if (outcome.status !== "signedIn") {
      throw new Error("expected a completed sign-in");
    }
    expect(outcome.session.sessionToken).toBe("s");
    expect(store.load()).toEqual(outcome.session);
    expect(calls[0]?.url).toBe(`${baseUrl}/api/v1/auth/login`);
    expect(calls[0]?.authorization).toBeNull();
    expect(JSON.parse(calls[0]?.body ?? "")).toEqual({ username: "kate", password: "hunter22" });
  });

  it("leaves a two-factor sign-in waiting and finishes it with a code", async () => {
    const store = new MemorySessionStore();
    const { fetch, calls } = scriptedFetch([
      () =>
        jsonResponse(200, {
          status: "secondFactorRequired",
          ticket: "ticket-1",
          methods: ["totp", "recoveryCode"],
        }),
      () =>
        jsonResponse(200, {
          userId: uuid,
          refreshToken: "r",
          sessionToken: "s",
          sessionTokenExpires: farFuture(),
          twoFactorEnrollmentRequired: false,
        }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const outcome = await client.login("kate", "hunter22");
    expect(outcome).toEqual({
      status: "secondFactorRequired",
      ticket: "ticket-1",
      methods: ["totp", "recoveryCode"],
    });
    expect(store.load()).toBeNull();
    const session = await client.completeSecondFactor("ticket-1", "totp", "123456");
    expect(session.sessionToken).toBe("s");
    expect(store.load()).toEqual(session);
    expect(calls[1]?.url).toBe(`${baseUrl}/api/v1/auth/login/second-factor`);
    expect(JSON.parse(calls[1]?.body ?? "")).toEqual({
      ticket: "ticket-1",
      method: "totp",
      code: "123456",
    });
  });

  it("flags the session when the server requires a second factor it lacks", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch } = scriptedFetch([() => problem(403, "twoFactorEnrollmentRequired")]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const seen: (boolean | undefined)[] = [];
    client.subscribe((session) => seen.push(session?.twoFactorEnrollmentRequired));
    const { error } = await client.api.GET("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
    });
    expect(error?.code).toBe("twoFactorEnrollmentRequired");
    expect(client.session?.twoFactorEnrollmentRequired).toBe(true);
    expect(seen).toEqual([true]);
    client.markEnrolled();
    expect(client.session?.twoFactorEnrollmentRequired).toBe(false);
  });

  it("leaves the session alone on other 403s", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch } = scriptedFetch([() => problem(403, "forbidden")]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    await client.api.GET("/api/v1/users/{user}", { params: { path: { user: "@me" } } });
    expect(client.session?.twoFactorEnrollmentRequired).toBeUndefined();
  });

  it("hands a passkey ceremony to the browser and claims it with the PKCE verifier", async () => {
    const store = new MemorySessionStore();
    let challenge = "";
    const { fetch, calls } = scriptedFetch([
      (r) => {
        const body = JSON.parse(r.body) as { handoff: { codeChallenge: string } };
        challenge = body.handoff.codeChallenge;
        return jsonResponse(201, {
          id: "cer-1",
          purpose: "signIn",
          options: {},
          expiresAt: farFuture(),
        });
      },
      () =>
        jsonResponse(200, {
          outcome: "signedIn",
          userId: uuid,
          refreshToken: "r",
          sessionToken: "s",
          sessionTokenExpires: farFuture(),
          twoFactorEnrollmentRequired: false,
        }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const opened: string[] = [];
    let disposed = false;
    const outcome = await client.runPasskeyCeremony(
      { purpose: "signIn" },
      {
        kind: "handoff",
        handoff: {
          prepare: () =>
            Promise.resolve({
              returnTo: "http://127.0.0.1:4000/passkey",
              open: (url: string) => {
                opened.push(url);
                return Promise.resolve({ ceremony: "cer-1", outcome: "done" as const });
              },
              dispose: () => {
                disposed = true;
              },
            }),
        },
      },
    );
    expect(opened).toEqual([`${baseUrl}/auth/passkey#ceremony=cer-1`]);
    expect(disposed).toBe(true);
    expect(outcome.outcome).toBe("signedIn");
    expect(store.load()?.sessionToken).toBe("s");
    const claim = JSON.parse(calls[1]?.body ?? "") as { codeVerifier: string };
    const digest = await crypto.subtle.digest(
      "SHA-256",
      new TextEncoder().encode(claim.codeVerifier),
    );
    expect(Buffer.from(digest).toString("base64url")).toBe(challenge);
    expect(calls[1]?.url).toBe(`${baseUrl}/api/v1/auth/passkey-ceremonies/cer-1/claim`);
  });

  it("treats a cancelled handoff as a cancellation and claims nothing", async () => {
    const { fetch, calls } = scriptedFetch([
      () =>
        jsonResponse(201, { id: "cer-2", purpose: "signIn", options: {}, expiresAt: farFuture() }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(
      client.runPasskeyCeremony(
        { purpose: "signIn" },
        {
          kind: "handoff",
          handoff: {
            prepare: () =>
              Promise.resolve({
                returnTo: "http://127.0.0.1:4000/passkey",
                open: () => Promise.resolve({ ceremony: "cer-2", outcome: "cancelled" as const }),
                dispose: () => undefined,
              }),
          },
        },
      ),
    ).rejects.toBeInstanceOf(PasskeyCancelledError);
    expect(calls).toHaveLength(1);
  });

  it("retries a read once after a short Retry-After, and leaves writes alone", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const limited = () =>
      new Response(JSON.stringify({ code: "rateLimited", title: "slow down", status: 429 }), {
        status: 429,
        headers: { "content-type": "application/problem+json", "retry-after": "2" },
      });
    const { fetch, calls } = scriptedFetch([
      limited,
      () => jsonResponse(200, { id: uuid, name: "kate", icon: null, onlineStatus: "online" }),
      limited,
    ]);
    const waits: number[] = [];
    const client = new AspenClient({
      baseUrl,
      sessionStore: store,
      fetch,
      sleep: (ms) => {
        waits.push(ms);
        return Promise.resolve();
      },
    });
    const { data } = await client.api.GET("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
    });
    expect(data?.name).toBe("kate");
    expect(waits).toEqual([2000]);
    const { error } = await client.api.POST("/api/v1/communities", { body: { name: "x" } });
    expect(error?.code).toBe("rateLimited");
    expect(calls).toHaveLength(3);
  });

  it("does not wait out a long Retry-After", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch, calls } = scriptedFetch([
      () =>
        new Response(JSON.stringify({ code: "rateLimited", title: "slow down", status: 429 }), {
          status: 429,
          headers: { "content-type": "application/problem+json", "retry-after": "60" },
        }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const { error } = await client.api.GET("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
    });
    expect(error?.code).toBe("rateLimited");
    expect(calls).toHaveLength(1);
  });

  it("registers an account without touching the session", async () => {
    const store = new MemorySessionStore();
    const { fetch, calls } = scriptedFetch([
      () => jsonResponse(201, { id: uuid, name: "kate", icon: null, onlineStatus: "offline" }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const user = await client.register("kate", "hunter22");
    expect(user.id).toBe(uuid);
    expect(store.load()).toBeNull();
    expect(calls[0]?.url).toBe(`${baseUrl}/api/v1/users`);
    expect(JSON.parse(calls[0]?.body ?? "")).toEqual({ name: "kate", password: "hunter22" });
  });

  it("reports a taken username by Problem code", async () => {
    const { fetch } = scriptedFetch([() => problem(409, "usernameTaken")]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(client.register("kate", "hunter22")).rejects.toMatchObject({
      code: "usernameTaken",
      status: 409,
    });
  });

  it("surfaces a login failure as an ApiProblemError carrying the Problem code", async () => {
    const { fetch } = scriptedFetch([() => problem(401, "invalidCredentials")]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await expect(client.login("kate", "wrong")).rejects.toMatchObject({
      name: "ApiProblemError",
      code: "invalidCredentials",
      status: 401,
    });
  });

  it("sends the session token as a bearer header", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch, calls } = scriptedFetch([
      () => jsonResponse(200, { id: uuid, name: "kate", icon: null, onlineStatus: "online" }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const { data } = await client.api.GET("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
    });
    expect(data?.name).toBe("kate");
    // openapi-fetch percent-encodes every path parameter; axum decodes it before matching.
    expect(calls[0]?.url).toBe(`${baseUrl}/api/v1/users/%40me`);
    expect(calls[0]?.authorization).toBe("Bearer session-1");
  });

  it("refreshes once and replays the request, body included, after a 401", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch, calls } = scriptedFetch([
      () => problem(401, "unauthorized"),
      () => jsonResponse(200, { sessionToken: "session-2", sessionTokenExpires: farFuture() }),
      () => jsonResponse(200, { id: uuid, name: "renamed", icon: null, onlineStatus: "online" }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const { data, response } = await client.api.PATCH("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
      body: { name: "renamed" },
    });
    expect(response.status).toBe(200);
    expect(data?.name).toBe("renamed");
    expect(calls.map((c) => c.method)).toEqual(["PATCH", "POST", "PATCH"]);
    expect(calls[1]?.url).toBe(`${baseUrl}/api/v1/auth/token-refresh`);
    expect(JSON.parse(calls[1]?.body ?? "")).toEqual({ refreshToken: "refresh-1" });
    expect(calls[2]?.authorization).toBe("Bearer session-2");
    expect(JSON.parse(calls[2]?.body ?? "")).toEqual({ name: "renamed" });
    expect(store.load()?.sessionToken).toBe("session-2");
  });

  it("clears the session when the refresh token is rejected", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const changes: (Session | null)[] = [];
    const { fetch } = scriptedFetch([
      () => problem(401, "unauthorized"),
      () => problem(401, "invalidToken"),
    ]);
    const client = new AspenClient({
      baseUrl,
      sessionStore: store,
      fetch,
      onSessionChange: (s) => changes.push(s),
    });
    const { error, response } = await client.api.GET("/api/v1/users/{user}", {
      params: { path: { user: "@me" } },
    });
    expect(response.status).toBe(401);
    expect(error?.code).toBe("unauthorized");
    expect(store.load()).toBeNull();
    expect(changes).toEqual([null]);
  });

  it("refreshes proactively when the session token is about to expire", async () => {
    const store = new MemorySessionStore();
    store.save({
      ...liveSession(),
      sessionTokenExpires: new Date(Date.now() + 10_000).toISOString(),
    });
    const { fetch, calls } = scriptedFetch([
      () => jsonResponse(200, { sessionToken: "session-2", sessionTokenExpires: farFuture() }),
      () => jsonResponse(200, []),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    await client.api.GET("/api/v1/users/{user}/communities", {
      params: { path: { user: "@me" } },
    });
    expect(calls.map((c) => c.url)).toEqual([
      `${baseUrl}/api/v1/auth/token-refresh`,
      `${baseUrl}/api/v1/users/%40me/communities`,
    ]);
    expect(calls[1]?.authorization).toBe("Bearer session-2");
  });

  it("hands out a session token for out-of-band use, refreshing when forced", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch, calls } = scriptedFetch([
      () => jsonResponse(200, { sessionToken: "session-2", sessionTokenExpires: farFuture() }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    expect(await client.freshSessionToken()).toBe("session-1");
    expect(calls).toHaveLength(0);
    expect(await client.freshSessionToken({ forceRefresh: true })).toBe("session-2");
    expect(calls).toHaveLength(1);
    store.clear();
    const loggedOut = new AspenClient({ baseUrl, sessionStore: store, fetch });
    expect(await loggedOut.freshSessionToken()).toBeNull();
  });

  it("logs out locally even when the server call fails", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch } = scriptedFetch([() => problem(500, "internal")]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    await client.logout();
    expect(store.load()).toBeNull();
  });

  it("sends array query parameters comma separated, as the OpenAPI document declares", async () => {
    const store = new MemorySessionStore();
    store.save(liveSession());
    const { fetch, calls } = scriptedFetch([
      () => jsonResponse(200, { data: { id: uuid, name: "Aspen", icon: null }, included: {} }),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
    const { data } = await client.api.GET("/api/v1/communities/{community}", {
      params: { path: { community: uuid }, query: { include: ["channels", "members"] } },
    });
    expect(calls[0]?.url).toBe(`${baseUrl}/api/v1/communities/${uuid}?include=channels,members`);
    expect(data?.included.channels).toBeUndefined();
  });

  it("exposes ApiProblemError for callers who prefer exceptions", () => {
    const error = new ApiProblemError({ code: "notFound", title: "gone", status: 404 });
    expect(error.message).toBe("gone");
    expect(error.code).toBe("notFound");
  });
});
