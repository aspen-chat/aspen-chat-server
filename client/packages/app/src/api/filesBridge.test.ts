import { beforeEach, describe, expect, it, vi } from "vitest";

/** What the native side was asked, in order. */
const calls: { method: string; options: Record<string, unknown> }[] = [];
let chosen: string | null = "doc-1";

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    getPlatform: () => "android",
    isPluginAvailable: (name: string) => name === "AspenFiles",
  },
  registerPlugin: () =>
    new Proxy(
      {},
      {
        get: (_, method: string) => (options: Record<string, unknown>) => {
          calls.push({ method, options });
          return Promise.resolve(method === "create" ? { id: chosen } : undefined);
        },
      },
    ),
}));

const { nativeCanChooseDestination, nativeChooseDestination } = await import("./filesBridge");

function decode(base64: string): Uint8Array {
  return Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
}

describe("filesBridge", () => {
  beforeEach(() => {
    calls.length = 0;
    chosen = "doc-1";
  });

  it("is used in the Android app", () => {
    expect(nativeCanChooseDestination()).toBe(true);
  });

  it("writes in pieces of about a megabyte that decode to what arrived, and closes", async () => {
    const sink = await nativeChooseDestination("notes.bin");
    expect(sink).not.toBeNull();
    const bytes = new Uint8Array(2_600_000).map((_, i) => (i * 7) % 256);
    for (let at = 0; at < bytes.length; at += 65_536) {
      await sink?.write(bytes.slice(at, at + 65_536).buffer);
    }
    await sink?.close();
    expect(calls[0]).toEqual({ method: "create", options: { name: "notes.bin" } });
    const writes = calls.filter((c) => c.method === "write");
    // Two full pieces, then the rest when the file closes.
    expect(writes).toHaveLength(3);
    const sizes = writes.map((w) => decode(w.options.data as string).length);
    expect(sizes[0]).toBeGreaterThanOrEqual(1024 * 1024);
    const joined = new Uint8Array(sizes.reduce((a, b) => a + b, 0));
    let at = 0;
    for (const w of writes) {
      const piece = decode(w.options.data as string);
      joined.set(piece, at);
      at += piece.length;
    }
    expect(joined).toEqual(bytes);
    expect(calls.at(-1)).toEqual({ method: "close", options: { id: "doc-1" } });
  });

  it("aborts without writing what it held", async () => {
    const sink = await nativeChooseDestination("partial.bin");
    await sink?.write(new Uint8Array(1000).buffer);
    await sink?.abort();
    expect(calls.map((c) => c.method)).toEqual(["create", "abort"]);
  });

  it("answers null when the user backs out of the picker", async () => {
    chosen = null;
    expect(await nativeChooseDestination("x.bin")).toBeNull();
  });
});
