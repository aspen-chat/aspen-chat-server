import { describe, expect, it } from "vitest";
import { WebStorageSessionStore, sessionTokenExpiresSoon, type Session } from "../src";

class FakeStorage implements Storage {
  #map = new Map<string, string>();
  get length(): number {
    return this.#map.size;
  }
  clear(): void {
    this.#map.clear();
  }
  getItem(key: string): string | null {
    return this.#map.get(key) ?? null;
  }
  key(index: number): string | null {
    return [...this.#map.keys()][index] ?? null;
  }
  removeItem(key: string): void {
    this.#map.delete(key);
  }
  setItem(key: string, value: string): void {
    this.#map.set(key, value);
  }
}

const session: Session = {
  userId: "u",
  refreshToken: "r",
  sessionToken: "s",
  sessionTokenExpires: "2030-01-01T00:00:00Z",
};

describe("WebStorageSessionStore", () => {
  it("round-trips a session and ignores garbage", () => {
    const storage = new FakeStorage();
    const store = new WebStorageSessionStore(storage);
    expect(store.load()).toBeNull();
    store.save(session);
    expect(store.load()).toEqual(session);
    storage.setItem("aspen.session", "{not json");
    expect(store.load()).toBeNull();
    storage.setItem("aspen.session", JSON.stringify({ userId: "u" }));
    expect(store.load()).toBeNull();
    store.clear();
    expect(storage.length).toBe(0);
  });
});

describe("sessionTokenExpiresSoon", () => {
  const now = Date.parse("2030-01-01T00:00:00Z");
  it("is true within the leeway and for unparseable timestamps", () => {
    expect(
      sessionTokenExpiresSoon(
        { ...session, sessionTokenExpires: "2030-01-01T00:00:30Z" },
        60_000,
        now,
      ),
    ).toBe(true);
    expect(
      sessionTokenExpiresSoon(
        { ...session, sessionTokenExpires: "2030-01-01T01:00:00Z" },
        60_000,
        now,
      ),
    ).toBe(false);
    expect(
      sessionTokenExpiresSoon({ ...session, sessionTokenExpires: "garbage" }, 60_000, now),
    ).toBe(true);
  });
});
