import { describe, expect, it } from "vitest";
import {
  AccountRegistry,
  MAX_ACCOUNTS,
  PRIMARY_ACCOUNT,
  accountSessionKey,
  foreignSessionKey,
} from "../src";

function memoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => {
      map.clear();
    },
    getItem: (key) => map.get(key) ?? null,
    key: (index) => [...map.keys()][index] ?? null,
    removeItem: (key) => {
      map.delete(key);
    },
    setItem: (key, value) => {
      map.set(key, value);
    },
  };
}

const HOME = "https://home.example";

function registry(storage = memoryStorage()): AccountRegistry {
  let next = 0;
  return new AccountRegistry(storage, () => `added-${String((next += 1))}`);
}

describe("AccountRegistry", () => {
  it("holds one account on the chosen server until turned on", () => {
    const accounts = registry();
    expect(accounts.state(HOME)).toEqual({
      enabled: false,
      front: PRIMARY_ACCOUNT,
      accounts: [{ id: PRIMARY_ACCOUNT, serverUrl: HOME }],
    });
    expect(accounts.state("https://other.example").accounts[0]?.serverUrl).toBe(
      "https://other.example",
    );
  });

  it("keeps added accounts and the one in front across instances", () => {
    const storage = memoryStorage();
    const first = registry(storage);
    first.enable(HOME);
    const added = first.draft("https://b.example");
    expect(first.add(HOME, added)).toBe(true);
    first.setFront(HOME, added.id);

    const state = registry(storage).state(HOME);
    expect(state.enabled).toBe(true);
    expect(state.front).toBe(added.id);
    expect(state.accounts.map((account) => account.serverUrl)).toEqual([HOME, "https://b.example"]);
  });

  it("refuses an account past the limit", () => {
    const accounts = registry();
    accounts.enable(HOME);
    for (let count = 1; count < MAX_ACCOUNTS; count += 1) {
      expect(accounts.add(HOME, accounts.draft(HOME))).toBe(true);
    }
    expect(accounts.add(HOME, accounts.draft(HOME))).toBe(false);
    expect(accounts.state(HOME).accounts).toHaveLength(MAX_ACCOUNTS);
  });

  it("moves the front off a removed account, and keeps the last one", () => {
    const accounts = registry();
    accounts.enable(HOME);
    const added = accounts.draft(HOME);
    accounts.add(HOME, added);
    accounts.setFront(HOME, added.id);
    accounts.remove(HOME, added.id);
    expect(accounts.state(HOME).front).toBe(PRIMARY_ACCOUNT);
    accounts.remove(HOME, PRIMARY_ACCOUNT);
    expect(accounts.state(HOME).accounts.map((account) => account.id)).toEqual([PRIMARY_ACCOUNT]);
  });

  it("keeps an added account alone when turned off, on the chosen server", () => {
    const accounts = registry();
    accounts.enable(HOME);
    const added = accounts.draft("https://b.example");
    accounts.add(HOME, added);
    accounts.disable(HOME, added.id);
    expect(accounts.state("https://b.example")).toEqual({
      enabled: false,
      front: added.id,
      accounts: [{ id: added.id, serverUrl: "https://b.example" }],
    });
  });

  it.each(["not json", "null", '{"version":2,"accounts":[]}', '{"version":1,"accounts":{}}'])(
    "reads %s as nothing kept",
    (kept) => {
      const storage = memoryStorage();
      storage.setItem("aspen.accounts", kept);
      expect(registry(storage).state(HOME).accounts).toEqual([
        { id: PRIMARY_ACCOUNT, serverUrl: HOME },
      ]);
    },
  );

  it("drops malformed and repeated entries", () => {
    const storage = memoryStorage();
    storage.setItem(
      "aspen.accounts",
      JSON.stringify({
        version: 1,
        enabled: true,
        front: "gone",
        accounts: [{ id: "a", serverUrl: HOME }, { id: "a", serverUrl: HOME }, { id: 3 }, null],
      }),
    );
    expect(registry(storage).state(HOME)).toEqual({
      enabled: true,
      front: "a",
      accounts: [{ id: "a", serverUrl: HOME }],
    });
  });
});

describe("session keys", () => {
  const primary = { id: PRIMARY_ACCOUNT, serverUrl: HOME };
  const added = { id: "local", serverUrl: HOME };

  it("leaves the first account's keys as they were", () => {
    expect(accountSessionKey(primary)).toBe("aspen.session");
    expect(foreignSessionKey(primary, "user", "b.example")).toBe("aspen.session.user@b.example");
  });

  it("names an added account's sessions by this install's id, whatever a home reports", () => {
    expect(foreignSessionKey(added, "user", "b.example")).toBe(
      foreignSessionKey(added, "another", "b.example"),
    );
    expect(foreignSessionKey(added, "user", "b.example")).not.toBe(
      foreignSessionKey(primary, "user", "b.example"),
    );
  });
});
