import { describe, expect, it } from "vitest";
import {
  CLOCK_OFFSET_MAX_MS,
  MemoryClockOffsetStore,
  ServerClock,
  WebStorageClockOffsetStore,
} from "../src";

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

const T = Date.parse("2026-11-11T12:00:00Z");

describe("ServerClock", () => {
  it("is the device's clock until a server says its time", () => {
    const clock = new ServerClock(() => T);
    expect(clock.offsetMs).toBe(0);
    expect(clock.heard).toBe(false);
    expect(clock.restored).toBe(false);
    expect(clock.now()).toBe(T);
    expect(clock.at(T + 5_000)).toBe(T + 5_000);
  });

  it("follows a server ahead of the device", () => {
    let device = T;
    const clock = new ServerClock(() => device);
    clock.note(new Date(T + 600_000).toISOString());
    expect(clock.heard).toBe(true);
    expect(clock.offsetMs).toBe(600_000);
    expect(clock.now()).toBe(T + 600_000);
    // The offset stays as the device's clock runs on.
    device = T + 1_000;
    expect(clock.now()).toBe(T + 601_000);
    expect(clock.at(T - 1_000)).toBe(T + 599_000);
  });

  it("follows a server behind the device", () => {
    const clock = new ServerClock(() => T);
    clock.note(new Date(T - 40_000).toISOString());
    expect(clock.offsetMs).toBe(-40_000);
    expect(clock.now()).toBe(T - 40_000);
  });

  it("counts from the device's time of receipt it is given", () => {
    const clock = new ServerClock(() => T);
    clock.note(new Date(T).toISOString(), T + 250);
    expect(clock.offsetMs).toBe(-250);
  });

  it("takes a server that gives no time, or no time it can read, to share the device's clock", () => {
    const clock = new ServerClock(() => T);
    clock.note(new Date(T + 600_000).toISOString());
    // A reconnect that reaches a server without the field forgets the offset.
    clock.note(null);
    expect(clock.heard).toBe(true);
    expect(clock.now()).toBe(T);
    clock.note(new Date(T + 600_000).toISOString());
    clock.note("not a time");
    expect(clock.offsetMs).toBe(0);
  });

  it("starts from the offset the last run saved, which is not a ready of this run", () => {
    const store = new MemoryClockOffsetStore();
    store.save(600_000);
    const clock = new ServerClock(() => T, store);
    expect(clock.offsetMs).toBe(600_000);
    expect(clock.now()).toBe(T + 600_000);
    expect(clock.restored).toBe(true);
    expect(clock.heard).toBe(false);
  });

  it("replaces a restored offset with the first ready's and saves that", () => {
    const store = new MemoryClockOffsetStore();
    store.save(600_000);
    const clock = new ServerClock(() => T, store);
    clock.note(new Date(T - 40_000).toISOString());
    expect(clock.offsetMs).toBe(-40_000);
    expect(clock.heard).toBe(true);
    expect(clock.restored).toBe(false);
    expect(store.load()).toBe(-40_000);
    // A deployment that gives no time is saved as agreeing with the device.
    clock.note(null);
    expect(store.load()).toBe(0);
    expect(new ServerClock(() => T, store).restored).toBe(true);
  });

  it("starts from zero when what is stored is no offset, or one too large to trust", () => {
    for (const stored of [
      Number.NaN,
      Number.POSITIVE_INFINITY,
      CLOCK_OFFSET_MAX_MS + 1,
      -CLOCK_OFFSET_MAX_MS - 1,
    ]) {
      const store = new MemoryClockOffsetStore();
      store.save(stored);
      const clock = new ServerClock(() => T, store);
      expect(clock.offsetMs).toBe(0);
      expect(clock.restored).toBe(false);
    }
    const store = new MemoryClockOffsetStore();
    store.save(-CLOCK_OFFSET_MAX_MS);
    expect(new ServerClock(() => T, store).offsetMs).toBe(-CLOCK_OFFSET_MAX_MS);
  });
});

describe("WebStorageClockOffsetStore", () => {
  it("round-trips an offset and reads anything else as none", () => {
    const storage = new FakeStorage();
    const store = new WebStorageClockOffsetStore(storage);
    expect(store.load()).toBeNull();
    store.save(-1_500);
    expect(store.load()).toBe(-1_500);
    for (const garbage of ["", "soon", '"600000"', "{}", "null", "[1]", "12abc"]) {
      storage.setItem("aspen.clock", garbage);
      expect(store.load()).toBeNull();
      expect(new ServerClock(() => T, store).offsetMs).toBe(0);
    }
  });

  it("keeps each deployment's offset under its own key", () => {
    const storage = new FakeStorage();
    const alpha = new WebStorageClockOffsetStore(storage, "aspen.clock.alpha.example");
    const beta = new WebStorageClockOffsetStore(storage, "aspen.clock.beta.example");
    new ServerClock(() => T, alpha).note(new Date(T + 600_000).toISOString());
    expect(new ServerClock(() => T, beta).offsetMs).toBe(0);
    new ServerClock(() => T, beta).note(new Date(T - 40_000).toISOString());
    expect(new ServerClock(() => T, alpha).offsetMs).toBe(600_000);
    expect(new ServerClock(() => T, beta).offsetMs).toBe(-40_000);
  });

  it("leaves the offset to this run when the storage refuses", () => {
    const storage = new FakeStorage();
    storage.setItem = () => {
      throw new Error("full");
    };
    storage.getItem = () => {
      throw new Error("refused");
    };
    const clock = new ServerClock(() => T, new WebStorageClockOffsetStore(storage));
    expect(clock.offsetMs).toBe(0);
    clock.note(new Date(T + 600_000).toISOString());
    expect(clock.offsetMs).toBe(600_000);
  });
});
