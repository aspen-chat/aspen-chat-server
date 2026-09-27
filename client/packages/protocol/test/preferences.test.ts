import { describe, expect, it } from "vitest";
import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  AspenClient,
  DEFAULT_DEVICE,
  MemorySessionStore,
  NOTIFICATION_OUTPUT,
  PreferenceStore,
  SAME_AS_VOICE,
  effectiveUserVolume,
  resolveDevice,
  userMuted,
  userVolume,
  type PreferenceDefinition,
  type PreferenceStorage,
} from "../src";

class FakeStorage implements PreferenceStorage {
  readonly items = new Map<string, string>();
  getItem(key: string) {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.items.set(key, value);
  }
  removeItem(key: string) {
    this.items.delete(key);
  }
}

const theme: PreferenceDefinition<"light" | "dark"> = {
  key: "look.theme",
  scope: "account",
  fallback: "light",
  parse: (raw) => (raw === "light" || raw === "dark" ? raw : undefined),
};

function accountClient(initial: Record<string, unknown>) {
  let values = initial;
  const calls: string[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init);
    calls.push(`${request.method} ${new URL(request.url).pathname}`);
    if (request.method === "PATCH") {
      const patch = (await request.json()) as Record<string, unknown>;
      values = Object.fromEntries(
        Object.entries({ ...values, ...patch }).filter(([, value]) => value !== null),
      );
    }
    return new Response(JSON.stringify({ values, updatedAt: "2026-09-26T00:00:00Z" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  const store = new MemorySessionStore();
  store.save({
    sessionToken: "s",
    sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
    refreshToken: "r",
    userId: "0190f0a0-0000-7000-8000-000000000001",
  });
  return {
    client: new AspenClient({ baseUrl: "http://api.example.org", sessionStore: store, fetch }),
    calls,
  };
}

describe("PreferenceStore", () => {
  it("keeps device preferences in the install's storage, across stores", async () => {
    const storage = new FakeStorage();
    const first = new PreferenceStore({ storage });
    expect(first.get(AUDIO_INPUT)).toBe(DEFAULT_DEVICE);
    expect(first.get(NOTIFICATION_OUTPUT)).toBe(SAME_AS_VOICE);
    let notified = 0;
    first.subscribe(() => {
      notified += 1;
    });
    await first.set(AUDIO_INPUT, { id: "mic-123", label: "Yeti" });
    await first.set(AUDIO_OUTPUT, { id: "spk-9", label: "Speakers" });
    expect(notified).toBe(2);
    expect(storage.items.get("aspen.preference.audio.input")).toBe(
      JSON.stringify({ id: "mic-123", label: "Yeti" }),
    );
    const second = new PreferenceStore({ storage });
    expect(second.get(AUDIO_INPUT)).toEqual({ id: "mic-123", label: "Yeti" });
    // the same stored value reads back as the same object, as a UI snapshot must
    expect(second.get(AUDIO_INPUT)).toBe(second.get(AUDIO_INPUT));
    expect(second.get(AUDIO_OUTPUT)).toEqual({ id: "spk-9", label: "Speakers" });
  });

  it("falls back when what is stored does not parse, and survives a broken storage", async () => {
    const storage = new FakeStorage();
    storage.items.set("aspen.preference.audio.input", "not json");
    storage.items.set("aspen.preference.audio.output", JSON.stringify(42));
    const store = new PreferenceStore({ storage });
    expect(store.get(AUDIO_INPUT)).toBe(DEFAULT_DEVICE);
    expect(store.get(AUDIO_OUTPUT)).toBe(DEFAULT_DEVICE);
    const broken: PreferenceStorage = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => undefined,
    };
    const session = new PreferenceStore({ storage: broken });
    await session.set(AUDIO_INPUT, { id: "mic-1", label: "One" });
    expect(session.get(AUDIO_INPUT)).toEqual({ id: "mic-1", label: "One" });
  });

  it("keeps a volume per other user, within bounds", async () => {
    const store = new PreferenceStore({ storage: null });
    const bob = userVolume("bob");
    expect(store.get(bob)).toBe(1);
    await store.set(bob, 1.75);
    expect(store.get(bob)).toBe(1.75);
    expect(store.get(userVolume("alice"))).toBe(1);
    expect(bob.key).toBe("voice.volume.bob");
    expect(bob.parse(3)).toBeUndefined();
    expect(bob.parse(-1)).toBeUndefined();
    expect(bob.parse("1")).toBeUndefined();
    expect(bob.parse(0)).toBe(0);
    // muting for oneself silences without losing the volume
    await store.set(userMuted("bob"), true);
    expect(effectiveUserVolume(store, "bob")).toBe(0);
    expect(store.get(bob)).toBe(1.75);
    await store.set(userMuted("bob"), false);
    expect(effectiveUserVolume(store, "bob")).toBe(1.75);
    expect(userMuted("bob").parse("yes")).toBeUndefined();
  });

  it("finds a remembered device by id, then by label once the id has changed", () => {
    const devices = [
      { deviceId: "abc", label: "Blue Yeti" },
      { deviceId: "def", label: "Headset" },
    ];
    expect(resolveDevice("default", devices)).toBeNull();
    expect(resolveDevice({ id: "def", label: "Old name" }, devices)).toBe("def");
    expect(resolveDevice({ id: "gone", label: "Blue Yeti" }, devices)).toBe("abc");
    expect(resolveDevice({ id: "gone", label: "Unplugged" }, devices)).toBeNull();
    expect(resolveDevice({ id: "gone", label: "" }, [{ deviceId: "x", label: "" }])).toBeNull();
  });

  it("loads account preferences from the server, writes them back as a patch, and forgets them at sign-out", async () => {
    const { client, calls } = accountClient({ "look.theme": "dark", "look.other": "x" });
    const store = new PreferenceStore({ storage: null, client });
    expect(store.get(theme)).toBe("light");
    await store.loadAccount();
    expect(store.get(theme)).toBe("dark");
    await store.set(theme, "light");
    expect(store.get(theme)).toBe("light");
    expect(calls).toEqual([
      "GET /api/v1/users/%40me/preferences",
      "PATCH /api/v1/users/%40me/preferences",
    ]);
    store.clearAccount();
    expect(store.get(theme)).toBe("light");
    await expect(new PreferenceStore({ storage: null }).set(theme, "dark")).rejects.toThrow(
      "server",
    );
  });
});
