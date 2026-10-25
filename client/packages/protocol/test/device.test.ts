import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AspenClient,
  DevicePuzzleRefusedError,
  MAX_PUZZLE_BITS,
  MemoryDeviceCredentialStore,
  MemorySessionStore,
  decodeChallenge,
  puzzleWork,
  solvePuzzle,
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
