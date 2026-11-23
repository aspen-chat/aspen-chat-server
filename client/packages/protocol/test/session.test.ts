import { describe, expect, it } from "vitest";
import {
  SessionVault,
  WebStorageSessionStore,
  sessionTokenExpiresSoon,
  type Session,
  type VaultBackend,
  type VaultRead,
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

class FakeBackend implements VaultBackend {
  contents: string | null = null;
  answer: VaultRead["status"] = "ok";
  writable = true;
  writes: string[] = [];
  gate: Promise<void> = Promise.resolve();

  read(): Promise<VaultRead> {
    return Promise.resolve(
      this.answer === "ok" ? { status: "ok", contents: this.contents } : { status: this.answer },
    );
  }

  async write(contents: string): Promise<boolean> {
    this.writes.push(contents);
    await this.gate;
    if (this.writable) {
      this.contents = contents;
    }
    return this.writable;
  }
}

async function opened(backend: VaultBackend, legacy?: Storage): Promise<SessionVault> {
  const opening = await SessionVault.open(backend, legacy);
  if (opening.status !== "open") {
    throw new Error(`the vault did not open: ${opening.status}`);
  }
  return opening.vault;
}

describe("SessionVault", () => {
  it("keeps sessions across openings, each under its own key", async () => {
    const backend = new FakeBackend();
    const vault = await opened(backend);
    vault.store().save(session);
    vault.store("aspen.session.a@b").save({ ...session, userId: "other" });
    await vault.settled();

    const again = await opened(backend);
    expect(again.store().load()).toEqual(session);
    expect(again.store("aspen.session.a@b").load()?.userId).toBe("other");
    again.store().clear();
    await again.settled();
    expect((await opened(backend)).store().load()).toBeNull();
  });

  it("writes in order, the latest replacing writes not begun", async () => {
    const backend = new FakeBackend();
    let release: () => void = () => undefined;
    backend.gate = new Promise((resolve) => {
      release = resolve;
    });
    const vault = await opened(backend);
    const store = vault.store();
    store.save({ ...session, sessionToken: "first" });
    store.save({ ...session, sessionToken: "second" });
    store.save({ ...session, sessionToken: "third" });
    release();
    await vault.settled();
    expect(backend.writes).toHaveLength(2);
    expect(JSON.parse(backend.contents ?? "")).toMatchObject({
      "aspen.session": { sessionToken: "third" },
    });
  });

  it("moves unencrypted sessions in, and removes them only once kept", async () => {
    const legacy = new FakeStorage();
    legacy.setItem("aspen.session", JSON.stringify(session));
    legacy.setItem("aspen.session.u@elsewhere", JSON.stringify({ ...session, userId: "abroad" }));
    legacy.setItem("aspen.sessionless", "kept");

    const refusing = new FakeBackend();
    refusing.writable = false;
    expect((await opened(refusing, legacy)).store().load()).toEqual(session);
    expect(legacy.getItem("aspen.session")).not.toBeNull();

    const backend = new FakeBackend();
    const vault = await opened(backend, legacy);
    expect(vault.store("aspen.session.u@elsewhere").load()?.userId).toBe("abroad");
    expect(legacy.getItem("aspen.session")).toBeNull();
    expect(legacy.getItem("aspen.session.u@elsewhere")).toBeNull();
    expect(legacy.getItem("aspen.sessionless")).toBe("kept");
  });

  it("prefers what it holds over an unencrypted copy, and removes the copy", async () => {
    const backend = new FakeBackend();
    backend.contents = JSON.stringify({ "aspen.session": { ...session, sessionToken: "kept" } });
    const legacy = new FakeStorage();
    legacy.setItem("aspen.session", JSON.stringify({ ...session, sessionToken: "stale" }));
    const vault = await opened(backend, legacy);
    expect(vault.store().load()?.sessionToken).toBe("kept");
    expect(legacy.getItem("aspen.session")).toBeNull();
  });

  it.each(["unavailable", "unreadable"] as const)(
    "reports %s and leaves unencrypted sessions to be moved later",
    async (status) => {
      const backend = new FakeBackend();
      backend.answer = status;
      const legacy = new FakeStorage();
      legacy.setItem("aspen.session", JSON.stringify(session));
      expect((await SessionVault.open(backend, legacy)).status).toBe(status);
      expect(legacy.getItem("aspen.session")).not.toBeNull();
      expect(backend.writes).toEqual([]);
    },
  );

  it("holds a session for the run when writing fails, and tries again on the next change", async () => {
    const backend = new FakeBackend();
    const vault = await opened(backend);
    backend.writable = false;
    vault.store().save(session);
    await vault.settled();
    expect(vault.store().load()).toEqual(session);
    backend.writable = true;
    vault.store("other").save(session);
    await vault.settled();
    expect(Object.keys(JSON.parse(backend.contents ?? "{}") as object)).toHaveLength(2);
  });

  it("writes nothing when kept in memory, and starts over when emptied", async () => {
    const memory = SessionVault.inMemory();
    memory.store().save(session);
    expect(memory.store().load()).toEqual(session);

    const backend = new FakeBackend();
    backend.contents = "ciphertext nobody can open";
    const vault = await SessionVault.emptied(backend);
    expect(vault.store().load()).toBeNull();
    expect(backend.contents).toBe("{}");
  });
});
