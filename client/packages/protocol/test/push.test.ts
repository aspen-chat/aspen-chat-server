import { describe, expect, it } from "vitest";
import {
  EMPTY_PUSH_STATE,
  RelayClient,
  base64UrlDecode,
  decryptWebPush,
  parsePointer,
  syncPush,
  type AspenClient,
  type PushDevice,
  type PushSource,
} from "../src";

describe("decryptWebPush", () => {
  it("reads RFC 8291's own example", async () => {
    const uaPublic = base64UrlDecode(
      "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
    );
    const b64 = (bytes: Uint8Array) =>
      btoa(String.fromCharCode(...bytes))
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replace(/=+$/, "");
    const keys = {
      privateKey: {
        kty: "EC",
        crv: "P-256",
        x: b64(uaPublic.slice(1, 33)),
        y: b64(uaPublic.slice(33, 65)),
        d: "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94",
      },
      publicKey: b64(uaPublic),
      auth: "BTBZMqHH6r4Tts7J_aSIgg",
    };
    const header = base64UrlDecode(
      "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
    );
    const ciphertext = base64UrlDecode(
      "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ",
    );
    const message = new Uint8Array([...header, ...ciphertext]);
    const plaintext = await decryptWebPush(message, keys);
    expect(new TextDecoder().decode(plaintext)).toBe("When I grow up, I want to be a watermelon");
  });
});

describe("parsePointer", () => {
  const read = (value: unknown) => parsePointer(new TextEncoder().encode(JSON.stringify(value)));

  it("reads the kinds it knows and ignores the rest", () => {
    expect(read({ v: 1, kind: "message", channel: "c", message: "m", badge: 2 })).toEqual({
      kind: "message",
      channel: "c",
      message: "m",
      badge: 2,
    });
    expect(read({ v: 1, kind: "deleted", channel: "c", message: "m", extra: true })).toEqual({
      kind: "deleted",
      channel: "c",
      message: "m",
    });
    expect(read({ v: 1, kind: "call", channel: "c", message: "m" })).toBeNull();
    expect(read({ v: 2, kind: "message", channel: "c", message: "m" })).toBeNull();
  });
});

/** A relay that remembers what it was asked. */
function fakeRelay() {
  const calls: string[] = [];
  let subscriptions = 0;
  const known = new Set<string>();
  const fetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = new URL(input instanceof Request ? input.url : input);
    const call = `${init?.method ?? "GET"} ${url.pathname}`;
    calls.push(call);
    const json = (status: number, body: unknown) =>
      Promise.resolve(new Response(body === null ? null : JSON.stringify(body), { status }));
    if (call === "POST /v1/devices") {
      known.add("d1");
      return json(201, { device: "d1", secret: "s1" });
    }
    if (call.startsWith("PATCH /v1/devices/")) {
      return known.has(url.pathname.split("/")[3] ?? "") ? json(204, null) : json(404, {});
    }
    if (call.endsWith("/subscriptions")) {
      subscriptions += 1;
      return json(201, {
        subscription: `sub${String(subscriptions)}`,
        endpoint: `https://relay.example/v1/push/p${String(subscriptions)}`,
      });
    }
    return json(204, null);
  };
  return { relay: new RelayClient("https://relay.example", fetch), calls, known };
}

function account(origin: string, userId: string, key: string | null, refreshToken = "r") {
  const registered: unknown[] = [];
  const client = {
    session: { userId, refreshToken, sessionToken: "s", sessionTokenExpires: "" },
    authMethods: () =>
      Promise.resolve({ push: key === null ? null : { applicationServerKey: key } }),
    registerPushSubscription: (body: unknown) => {
      registered.push(body);
      return Promise.resolve({ id: `dep-${userId}`, endpoint: "", createdAt: "" });
    },
  } as unknown as AspenClient;
  return { source: { origin, client } satisfies PushSource, registered };
}

const phone: PushDevice = {
  platform: "apns",
  app: "org.aspen.chat",
  environment: "production",
  token: "t1",
};

describe("syncPush", () => {
  it("registers the device and subscribes each account whose deployment wakes phones", async () => {
    const { relay, calls } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A");
    const quiet = account("https://b.example", "u2", null);
    const state = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home.source, quiet.source]);
    expect(calls).toEqual(["POST /v1/devices", "POST /v1/devices/d1/subscriptions"]);
    expect(state.device).toEqual({ id: "d1", secret: "s1", token: "t1" });
    expect(state.accounts.map((a) => [a.origin, a.subscription, a.deploymentSubscription])).toEqual(
      [["https://a.example", "sub1", "dep-u1"]],
    );
    const [sent] = home.registered as { endpoint: string; p256dh: string; auth: string }[];
    expect(sent?.endpoint).toBe("https://relay.example/v1/push/p1");
    expect(base64UrlDecode(sent?.p256dh ?? "")).toHaveLength(65);
    expect(base64UrlDecode(sent?.auth ?? "")).toHaveLength(16);
  });

  it("keeps what is current, and subscribes again for a new key or a new sign-in", async () => {
    const { relay, calls } = fakeRelay();
    const first = await syncPush(relay, EMPTY_PUSH_STATE, phone, [
      account("https://a.example", "u1", "KEY-A").source,
    ]);
    calls.length = 0;
    const same = await syncPush(relay, first, phone, [
      account("https://a.example", "u1", "KEY-A").source,
    ]);
    expect(calls).toEqual([]);
    expect(same.accounts).toEqual(first.accounts);

    const rekeyed = await syncPush(relay, same, phone, [
      account("https://a.example", "u1", "KEY-B").source,
    ]);
    expect(calls).toEqual([
      "POST /v1/devices/d1/subscriptions",
      "DELETE /v1/devices/d1/subscriptions/sub1",
    ]);
    expect(rekeyed.accounts[0]?.applicationServerKey).toBe("KEY-B");

    calls.length = 0;
    const signedOut = await syncPush(relay, rekeyed, phone, []);
    expect(calls).toEqual(["DELETE /v1/devices/d1/subscriptions/sub2"]);
    expect(signedOut.accounts).toEqual([]);
  });

  it("follows a new platform token, and starts again when the relay forgot the device", async () => {
    const { relay, calls, known } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A").source;
    const first = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home]);
    calls.length = 0;
    const moved = await syncPush(relay, first, { ...phone, token: "t2" }, [home]);
    expect(calls).toEqual(["PATCH /v1/devices/d1"]);
    expect(moved.device?.token).toBe("t2");
    known.clear();
    calls.length = 0;
    const again = await syncPush(relay, moved, { ...phone, token: "t3" }, [home]);
    expect(calls).toEqual([
      "PATCH /v1/devices/d1",
      "POST /v1/devices",
      "POST /v1/devices/d1/subscriptions",
    ]);
    expect(again.accounts).toHaveLength(1);
  });
});
