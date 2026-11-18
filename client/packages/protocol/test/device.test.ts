import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AspenClient,
  DevicePuzzleRefusedError,
  MAX_PUZZLE_BITS,
  MemoryDeviceCredentialStore,
  MemorySessionStore,
  UnreachableError,
  decodeChallenge,
  puzzleRanges,
  puzzleWork,
  solvePuzzle,
  solvePuzzleRange,
  type Session,
} from "../src";

const vectors = JSON.parse(
  readFileSync(new URL("../../../../spec/device_puzzle_vectors.json", import.meta.url), "utf8"),
) as { vectors: { challenge: string; nonce: number; zeroBits: number }[] };

const baseUrl = "https://aspen.test";

function json(status: number, body: unknown, problem = false): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": problem ? "application/problem+json" : "application/json" },
  });
}

function problem(status: number, code: string): Response {
  return json(status, { code, title: code, status }, true);
}

interface Sent {
  path: string;
  device: string | null;
  authorization: string | null;
  body: string;
}

/** A scripted fetch: each call pops the next responder and records what it was sent. */
function scripted(responders: ((sent: Sent) => Response)[]) {
  const calls: Sent[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const request = new Request(input, init);
    const sent: Sent = {
      path: new URL(request.url).pathname,
      device: request.headers.get("aspen-device"),
      authorization: request.headers.get("authorization"),
      body: await request.text(),
    };
    calls.push(sent);
    const responder = responders.shift();
    if (responder === undefined) {
      throw new Error(`unexpected request to ${sent.path}`);
    }
    return responder(sent);
  };
  return { fetch, calls };
}

const methods = {
  passkeys: null,
  twoFactorRequired: false,
  registrationInviteRequired: false,
  federationDomain: null,
};

describe("the device puzzle", () => {
  it("passes the vectors the server passes", () => {
    for (const vector of vectors.vectors) {
      expect(puzzleWork(decodeChallenge(vector.challenge), vector.nonce), vector.challenge).toBe(
        vector.zeroBits,
      );
    }
  });

  it("solves a puzzle with its first solution", async () => {
    const challenge = new TextEncoder().encode("challenge");
    const nonce = await solvePuzzle(challenge, 10);
    expect(puzzleWork(challenge, nonce)).toBeGreaterThanOrEqual(10);
    for (let earlier = 0; earlier < nonce; earlier++) {
      expect(puzzleWork(challenge, earlier)).toBeLessThan(10);
    }
  });

  it("stops once told to", async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(solvePuzzle(new Uint8Array([1]), 32, controller.signal)).rejects.toBeDefined();
  });
});

describe("puzzleRanges", () => {
  it("covers every safe nonce once, in order, for any number of parts", () => {
    for (const parts of [1, 2, 3, 7, 8, 64, 1000]) {
      const ranges = puzzleRanges(parts);
      expect(ranges).toHaveLength(parts);
      expect(ranges[0]?.from).toBe(0);
      expect(ranges.at(-1)?.to).toBe(Number.MAX_SAFE_INTEGER);
      for (const [index, range] of ranges.entries()) {
        expect(Number.isSafeInteger(range.from) && Number.isSafeInteger(range.to)).toBe(true);
        expect(range.to).toBeGreaterThan(range.from);
        if (index > 0) {
          expect(range.from).toBe(ranges[index - 1]?.to);
        }
      }
    }
  });

  it("refuses a count of parts that is not a positive integer", () => {
    for (const parts of [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(() => puzzleRanges(parts)).toThrow(RangeError);
    }
  });

  it("gives each part a range that holds solutions", () => {
    const challenge = new TextEncoder().encode("challenge");
    for (const { from, to } of puzzleRanges(8)) {
      const nonce = solvePuzzleRange(challenge, 10, from, to);
      expect(nonce).not.toBeNull();
      expect(nonce).toBeGreaterThanOrEqual(from);
      expect(nonce).toBeLessThan(to);
      expect(puzzleWork(challenge, nonce ?? 0)).toBeGreaterThanOrEqual(10);
    }
  });
});

describe("AspenClient's device credential", () => {
  it("earns one when refused for lacking it, and sends the request again", async () => {
    const challenge = "Y2hhbGxlbmdl";
    const { fetch, calls } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      () => json(200, { challenge, difficulty: 4, expiresAt: new Date().toISOString() }),
      () => json(200, { credential: "device-1", expiresAt: new Date().toISOString() }),
      () => json(200, methods),
    ]);
    const devices = new MemoryDeviceCredentialStore();
    const client = new AspenClient({
      baseUrl,
      sessionStore: new MemorySessionStore(),
      deviceStore: devices,
      fetch,
    });
    const answer = await client.api.GET("/api/v1/auth/methods");
    expect(answer.response.status).toBe(200);
    expect(calls.map((call) => call.path)).toEqual([
      "/api/v1/auth/methods",
      "/api/v1/auth/device-challenges",
      "/api/v1/auth/device-credentials",
      "/api/v1/auth/methods",
    ]);
    const redeemed = JSON.parse(calls[2]?.body ?? "{}") as { challenge: string; nonce: number };
    expect(redeemed.challenge).toBe(challenge);
    expect(puzzleWork(decodeChallenge(challenge), redeemed.nonce)).toBeGreaterThanOrEqual(4);
    expect(calls[3]?.device).toBe("device-1");
    expect(devices.load()).toBe("device-1");
  });

  it("presents the one it holds, keeps a renewal, and sends none with a session", async () => {
    const { fetch, calls } = scripted([
      () =>
        new Response(JSON.stringify(methods), {
          status: 200,
          headers: { "content-type": "application/json", "aspen-device-credential": "device-2" },
        }),
      () => json(200, methods),
    ]);
    const devices = new MemoryDeviceCredentialStore();
    devices.save("device-1");
    const sessions = new MemorySessionStore();
    const client = new AspenClient({
      baseUrl,
      sessionStore: sessions,
      deviceStore: devices,
      fetch,
    });
    await client.api.GET("/api/v1/auth/methods");
    expect(calls[0]?.device).toBe("device-1");
    expect(devices.load()).toBe("device-2");
    const session: Session = {
      userId: "0190f0a0-0000-7000-8000-000000000001",
      refreshToken: "refresh-1",
      sessionToken: "session-1",
      sessionTokenExpires: new Date(Date.now() + 3 * 60 * 60 * 1000).toISOString(),
    };
    sessions.save(session);
    const signedIn = new AspenClient({
      baseUrl,
      sessionStore: sessions,
      deviceStore: devices,
      fetch,
    });
    await signedIn.api.GET("/api/v1/auth/methods");
    expect(calls[1]?.device).toBeNull();
    expect(calls[1]?.authorization).toBe("Bearer session-1");
  });

  it("asks for another challenge when the first expired while it was solved", async () => {
    const challenge = "Y2hhbGxlbmdl";
    const asked = () =>
      json(200, { challenge, difficulty: 1, expiresAt: new Date().toISOString() });
    const { fetch, calls } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      asked,
      () => problem(400, "deviceChallengeInvalid"),
      asked,
      () => json(200, { credential: "device-1", expiresAt: new Date().toISOString() }),
      () => json(200, methods),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const answer = await client.api.GET("/api/v1/auth/methods");
    expect(answer.response.status).toBe(200);
    expect(calls).toHaveLength(6);
  });

  it("gives up on a deployment asking for a puzzle harder than it solves", async () => {
    const { fetch, calls } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      () =>
        json(200, {
          challenge: "Y2hhbGxlbmdl",
          difficulty: MAX_PUZZLE_BITS + 1,
          expiresAt: new Date().toISOString(),
        }),
    ]);
    let asked = false;
    const client = new AspenClient({
      baseUrl,
      sessionStore: new MemorySessionStore(),
      fetch,
      solvePuzzle: () => {
        asked = true;
        return Promise.resolve(0);
      },
    });
    await expect(client.api.GET("/api/v1/auth/methods")).rejects.toBeInstanceOf(
      DevicePuzzleRefusedError,
    );
    expect(asked).toBe(false);
    expect(calls).toHaveLength(2);
  });

  describe("with a puzzle that takes too long", () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it("stops solving and gives the deployment up", async () => {
      vi.useFakeTimers();
      const { fetch, calls } = scripted([
        () => problem(401, "deviceCredentialRequired"),
        () =>
          json(200, {
            challenge: "Y2hhbGxlbmdl",
            difficulty: 20,
            expiresAt: new Date().toISOString(),
          }),
      ]);
      let stopped = false;
      const client = new AspenClient({
        baseUrl,
        sessionStore: new MemorySessionStore(),
        fetch,
        // A solver that never finishes, and notices when it is told to stop.
        solvePuzzle: (_challenge, _difficulty, signal) =>
          new Promise<number>(() => {
            signal?.addEventListener("abort", () => {
              stopped = true;
            });
          }),
      });
      const request = client.api.GET("/api/v1/auth/methods");
      const settled = expect(request).rejects.toBeInstanceOf(DevicePuzzleRefusedError);
      await vi.advanceTimersByTimeAsync(60_000);
      await settled;
      expect(stopped).toBe(true);
      expect(calls).toHaveLength(2);
    });
  });

  it("gives the caller the failed step's answer, not the refusal that sent it to earn one", async () => {
    const { fetch, calls } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      () => problem(429, "rateLimited"),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const answer = await client.api.POST("/api/v1/auth/login", {
      body: { username: "kate", password: "hunter22" },
    });
    expect(answer.response.status).toBe(429);
    expect(answer.error).toMatchObject({ code: "rateLimited" });
    expect(calls.map((call) => call.path)).toEqual([
      "/api/v1/auth/login",
      "/api/v1/auth/device-challenges",
    ]);
  });

  it("gives a redemption's refusal, past an expired challenge asked for again", async () => {
    const challenge = "Y2hhbGxlbmdl";
    const asked = () =>
      json(200, { challenge, difficulty: 1, expiresAt: new Date().toISOString() });
    const { fetch } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      asked,
      () => problem(400, "deviceChallengeInvalid"),
      asked,
      () => problem(503, "serverBusy"),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const answer = await client.api.GET("/api/v1/auth/methods");
    expect(answer.response.status).toBe(503);
    expect(answer.error).toMatchObject({ code: "serverBusy" });
  });

  it("gives every caller refused together its own copy of a failed step's answer", async () => {
    const { fetch } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      () => problem(401, "deviceCredentialRequired"),
      () => problem(503, "serverBusy"),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const answers = await Promise.all([
      client.api.GET("/api/v1/auth/methods"),
      client.api.GET("/api/v1/auth/methods"),
    ]);
    expect(answers.map((answer) => answer.error)).toMatchObject([
      { code: "serverBusy" },
      { code: "serverBusy" },
    ]);
  });

  it("says the deployment could not be reached when a step gets no answer", async () => {
    const { fetch } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      () => {
        throw new TypeError("Failed to fetch");
      },
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    const failure = await client.api.GET("/api/v1/auth/methods").catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(UnreachableError);
    expect((failure as UnreachableError).server).toBe(baseUrl);
    expect((failure as UnreachableError).cause).toBeInstanceOf(TypeError);
  });

  it("leaves a request stopped by its signal as stopped, not unreachable", async () => {
    const stopped = new DOMException("stopped", "AbortError");
    const client = new AspenClient({
      baseUrl,
      sessionStore: new MemorySessionStore(),
      fetch: () => Promise.reject(stopped),
    });
    await expect(client.api.GET("/api/v1/auth/methods")).rejects.toBe(stopped);
  });

  it("earns one credential for requests refused together", async () => {
    const challenge = "Y2hhbGxlbmdl";
    const { fetch, calls } = scripted([
      () => problem(401, "deviceCredentialRequired"),
      () => problem(401, "deviceCredentialRequired"),
      () => json(200, { challenge, difficulty: 1, expiresAt: new Date().toISOString() }),
      () => json(200, { credential: "device-1", expiresAt: new Date().toISOString() }),
      () => json(200, methods),
      () => json(200, methods),
    ]);
    const client = new AspenClient({ baseUrl, sessionStore: new MemorySessionStore(), fetch });
    await Promise.all([
      client.api.GET("/api/v1/auth/methods"),
      client.api.GET("/api/v1/auth/methods"),
    ]);
    expect(calls.filter((call) => call.path === "/api/v1/auth/device-challenges")).toHaveLength(1);
  });
});
