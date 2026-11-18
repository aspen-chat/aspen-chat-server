import { describe, expect, it } from "vitest";
import {
  EMPTY_PUSH_STATE,
  RelayClient,
  RelayError,
  base64UrlDecode,
  decryptWebPush,
  parsePointer,
  stopPush,
  syncPush,
  type AspenClient,
  type PushDevice,
  type PushSource,
  type PushState,
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
    expect(read({ v: 1, kind: "notice", channel: "c", notice: "n", badge: 1 })).toEqual({
      kind: "notice",
      channel: "c",
      notice: "n",
      badge: 1,
    });
    expect(read({ v: 1, kind: "notice", channel: "c", message: "m" })).toBeNull();
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
  const deleted: string[] = [];
  const client = {
    session: { userId, refreshToken, sessionToken: "s", sessionTokenExpires: "" },
    authMethods: () =>
      Promise.resolve({ push: key === null ? null : { applicationServerKey: key } }),
    registerPushSubscription: (body: unknown) => {
      registered.push(body);
      return Promise.resolve({ id: `dep-${userId}`, endpoint: "", createdAt: "" });
    },
    deletePushSubscription: (id: string) => {
      deleted.push(id);
      return Promise.resolve();
    },
  } as unknown as AspenClient;
  return { source: { origin, client } satisfies PushSource, registered, deleted };
}

const phone: PushDevice = {
  platform: "apns",
  app: "com.aspen-chat.client",
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
    expect(state.device).toEqual({
      id: "d1",
      secret: "s1",
      token: "t1",
      platform: "apns",
      app: "com.aspen-chat.client",
      environment: "production",
    });
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

  it("keeps an account's subscription when its deployment cannot be asked", async () => {
    const { relay, calls } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A");
    const first = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home.source]);
    calls.length = 0;
    const unreachable = {
      origin: home.source.origin,
      client: {
        session: home.source.client.session,
        authMethods: () => Promise.reject(new TypeError("Failed to fetch")),
      } as unknown as AspenClient,
    } satisfies PushSource;
    const kept = await syncPush(relay, first, phone, [unreachable]);
    expect(calls).toEqual([]);
    expect(kept.accounts).toEqual(first.accounts);
    // A deployment that answers, and no longer wakes phones, is unsubscribed.
    const off = await syncPush(relay, kept, phone, [
      account("https://a.example", "u1", null).source,
    ]);
    expect(calls).toEqual(["DELETE /v1/devices/d1/subscriptions/sub1"]);
    expect(off.accounts).toEqual([]);
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

describe("syncPush, when the phone is registered as something else", () => {
  it("ends the device and registers it anew when its environment changes", async () => {
    const { relay, calls } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A");
    const first = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home.source]);
    calls.length = 0;
    const sandbox = { ...phone, environment: "sandbox" as const, token: "t2" };
    const again = await syncPush(relay, first, sandbox, [home.source]);
    expect(home.deleted).toEqual(["dep-u1"]);
    expect(calls).toEqual([
      "DELETE /v1/devices/d1",
      "POST /v1/devices",
      "POST /v1/devices/d1/subscriptions",
    ]);
    expect(again.device?.environment).toBe("sandbox");
    expect(again.device?.token).toBe("t2");
    expect(again.accounts.map((a) => a.subscription)).toEqual(["sub2"]);
  });

  it("registers anew a device kept before what it was registered as was", async () => {
    const { relay, calls } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A");
    const first = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home.source]);
    const { id, secret, token } = first.device ?? { id: "", secret: "", token: "" };
    const kept: PushState = { ...first, device: { id, secret, token } };
    calls.length = 0;
    const again = await syncPush(relay, kept, phone, [home.source]);
    expect(calls).toEqual([
      "DELETE /v1/devices/d1",
      "POST /v1/devices",
      "POST /v1/devices/d1/subscriptions",
    ]);
    expect(again.device?.environment).toBe("production");
  });

  it("changes nothing when the relay will not end the device", async () => {
    const failing = new RelayClient("https://relay.example", () =>
      Promise.resolve(new Response(JSON.stringify({}), { status: 503 })),
    );
    const state: PushState = {
      version: 1,
      device: { id: "d1", secret: "s1", token: "t1", platform: "apns", app: phone.app },
      accounts: [],
    };
    await expect(syncPush(failing, state, phone, [])).rejects.toBeInstanceOf(RelayError);
  });
});

describe("stopPush", () => {
  it("deletes each deployment's subscription and then the device", async () => {
    const { relay, calls } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A");
    const away = account("https://b.example", "u2", "KEY-B");
    const state = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home.source, away.source]);
    calls.length = 0;
    // A deployment no longer signed in to is left to find the relay's subscription gone.
    const stopped = await stopPush(relay, state, [home.source]);
    expect(home.deleted).toEqual(["dep-u1"]);
    expect(away.deleted).toEqual([]);
    expect(calls).toEqual(["DELETE /v1/devices/d1"]);
    expect(stopped).toEqual(EMPTY_PUSH_STATE);
  });

  it("goes on past a deployment that cannot be reached", async () => {
    const { relay, calls } = fakeRelay();
    const home = account("https://a.example", "u1", "KEY-A");
    const state = await syncPush(relay, EMPTY_PUSH_STATE, phone, [home.source]);
    calls.length = 0;
    const unreachable = {
      origin: home.source.origin,
      client: {
        session: home.source.client.session,
        deletePushSubscription: () => Promise.reject(new TypeError("Failed to fetch")),
      } as unknown as AspenClient,
    } satisfies PushSource;
    expect(await stopPush(relay, state, [unreachable])).toEqual(EMPTY_PUSH_STATE);
    expect(calls).toEqual(["DELETE /v1/devices/d1"]);
  });

  it("refuses to forget a device the relay would not end", async () => {
    const failing = new RelayClient("https://relay.example", () =>
      Promise.resolve(new Response(JSON.stringify({}), { status: 503 })),
    );
    const state: PushState = {
      version: 1,
      device: { id: "d1", secret: "s1", token: "t1" },
      accounts: [],
    };
    await expect(stopPush(failing, state, [])).rejects.toBeInstanceOf(RelayError);
  });

  it("has nothing to end for a phone never registered", async () => {
    const { relay, calls } = fakeRelay();
    expect(await stopPush(relay, EMPTY_PUSH_STATE, [])).toEqual(EMPTY_PUSH_STATE);
    expect(calls).toEqual([]);
  });
});
