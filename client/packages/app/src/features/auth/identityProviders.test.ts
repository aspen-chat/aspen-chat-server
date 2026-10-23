import { afterEach, describe, expect, it, vi } from "vitest";
import {
  FLOW_KEPT_MS,
  keepFlow,
  providerHandoff,
  takeFlow,
  type StartedFlow,
} from "./identityProviders";
import { mobileHandoff } from "./passkeyTransport";

/** `sessionStorage` as a page has it, in memory. */
class MemoryStorage implements Storage {
  #items = new Map<string, string>();
  get length() {
    return this.#items.size;
  }
  clear() {
    this.#items.clear();
  }
  getItem(key: string) {
    return this.#items.get(key) ?? null;
  }
  key(index: number) {
    return [...this.#items.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.#items.delete(key);
  }
  setItem(key: string, value: string) {
    this.#items.set(key, value);
  }
}

/** A browser whose storage is turned off: every use throws. */
class RefusingStorage extends MemoryStorage {
  override setItem(): void {
    throw new DOMException("refused", "SecurityError");
  }
  override getItem(): string | null {
    throw new DOMException("refused", "SecurityError");
  }
}

function started(at: number, verifier = "verifier"): StartedFlow {
  return {
    verifier,
    purpose: "signIn",
    providerName: "Example Corp",
    returnPath: "/invite/abc?at=chat.example.org",
    startedAt: at,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("flows kept for the return page", () => {
  it("are taken once", () => {
    const storage = new MemoryStorage();
    expect(keepFlow("flow-1", started(1_000), storage)).toBe(true);
    expect(takeFlow("flow-1", storage)).toEqual(started(1_000));
    expect(takeFlow("flow-1", storage)).toBeNull();
    expect(takeFlow("flow-2", storage)).toBeNull();
  });

  it("drop those the server has forgotten, and anything unreadable, as another starts", () => {
    const storage = new MemoryStorage();
    keepFlow("old", started(0), storage);
    keepFlow("recent", started(FLOW_KEPT_MS / 2), storage);
    storage.setItem("aspen.identityProviderFlow.broken", "{not json");
    storage.setItem("aspen.other", "kept");
    keepFlow("new", started(FLOW_KEPT_MS + 1), storage);
    expect(takeFlow("old", storage)).toBeNull();
    expect(storage.getItem("aspen.identityProviderFlow.broken")).toBeNull();
    expect(takeFlow("recent", storage)?.startedAt).toBe(FLOW_KEPT_MS / 2);
    expect(takeFlow("new", storage)?.startedAt).toBe(FLOW_KEPT_MS + 1);
    expect(storage.getItem("aspen.other")).toBe("kept");
  });

  it("say so where the browser keeps nothing", () => {
    const storage = new RefusingStorage();
    expect(keepFlow("flow-1", started(1), storage)).toBe(false);
    expect(takeFlow("flow-1", storage)).toBeNull();
  });
});

describe("providerHandoff", () => {
  it("sends the web client's own tab, and the phone through the passkey hand-off", () => {
    expect(providerHandoff("web", "https://chat.example.org")).toBeNull();
    expect(providerHandoff("mobile", "https://chat.example.org")).toBe(mobileHandoff);
  });

  it("opens the provider from the desktop's listener, naming the server to finish on", async () => {
    // Without the preload bridge the desktop shell cannot listen for the return.
    expect(providerHandoff("desktop", "https://chat.example.org")).toBeNull();
    const returned = { ceremony: "flow-1", outcome: "done" as const, code: "back" };
    const bridge = {
      prepare: vi.fn(() =>
        Promise.resolve({ id: "h1", returnTo: "http://127.0.0.1:4000/passkey" }),
      ),
      open: vi.fn(),
      openProvider: vi.fn(() => Promise.resolve(returned)),
      dispose: vi.fn(),
    };
    vi.stubGlobal("window", { aspenDesktop: { passkeyHandoff: bridge } });
    const handoff = providerHandoff("desktop", "https://chat.example.org");
    const session = await handoff?.prepare();
    expect(session?.returnTo).toBe("http://127.0.0.1:4000/passkey");
    expect(await session?.open("https://idp.example.org/authorize")).toEqual(returned);
    expect(bridge.openProvider).toHaveBeenCalledWith(
      "h1",
      "https://idp.example.org/authorize",
      "https://chat.example.org",
    );
    expect(bridge.open).not.toHaveBeenCalled();
    session?.dispose();
    expect(bridge.dispose).toHaveBeenCalledWith("h1");
  });
});
