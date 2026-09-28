import { describe, expect, it } from "vitest";
import { windowParts } from "@/features/messages/blocked";

describe("windowParts", () => {
  it("collapses consecutive blocked messages into runs and keeps the rest", () => {
    const blocked = new Set(["b", "c", "e"]);
    expect(windowParts(["a", "b", "c", "d", "e"], (id) => blocked.has(id))).toEqual([
      { kind: "message", id: "a", index: 0 },
      { kind: "blocked", ids: ["b", "c"], index: 1 },
      { kind: "message", id: "d", index: 3 },
      { kind: "blocked", ids: ["e"], index: 4 },
    ]);
  });

  it("leaves a window with nothing blocked as it is", () => {
    expect(windowParts(["a", "b"], () => false)).toEqual([
      { kind: "message", id: "a", index: 0 },
      { kind: "message", id: "b", index: 1 },
    ]);
  });
});
