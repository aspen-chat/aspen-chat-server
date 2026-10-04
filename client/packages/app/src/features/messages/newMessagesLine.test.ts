import { describe, expect, it } from "vitest";
import { lineAt } from "@/features/messages/newMessagesLine";

const window = (ids: string[], hasOlder = false) => ({ ids, hasOlder, atLatest: true });

describe("lineAt", () => {
  it("goes under the last message at or before the read position", () => {
    expect(lineAt(window(["a", "c", "e"]), "c")).toBe(1);
    expect(lineAt(window(["a", "c", "e"]), "d")).toBe(1);
  });

  it("is at the top of a channel whose every message is new", () => {
    expect(lineAt(window(["b", "c"]), "a")).toBe(-1);
  });

  it("is nowhere when nothing was unread", () => {
    expect(lineAt(window(["a", "b"]), null)).toBeNull();
  });

  it("is nowhere when nothing after the read position is loaded", () => {
    expect(lineAt(window(["a", "b"]), "b")).toBeNull();
    expect(lineAt(window(["a", "b"]), "c")).toBeNull();
  });

  it("is nowhere when it belongs above what is loaded", () => {
    expect(lineAt(window(["b", "c"], true), "a")).toBeNull();
  });
});
