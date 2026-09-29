import { describe, expect, it } from "vitest";
import { mergeResults } from "./merge";

const at = (id: string, minute: number) => ({
  id,
  timestamp: new Date(Date.UTC(2026, 8, 1, 12, minute)).toISOString(),
});

describe("mergeResults", () => {
  it("interleaves deployments newest first", () => {
    const merged = mergeResults([
      { key: "home", messages: [at("h2", 50), at("h1", 10)], exhausted: true },
      { key: "b", messages: [at("b2", 40), at("b1", 20)], exhausted: true },
    ]);
    expect(merged.map((r) => r.message.id)).toEqual(["h2", "b2", "b1", "h1"]);
    expect(merged[1]?.key).toBe("b");
  });

  it("holds back what a deployment with more to give might still precede", () => {
    const merged = mergeResults([
      { key: "home", messages: [at("h2", 50), at("h1", 10)], exhausted: true },
      { key: "b", messages: [at("b2", 40), at("b1", 30)], exhausted: false },
    ]);
    expect(merged.map((r) => r.message.id)).toEqual(["h2", "b2", "b1"]);
  });
});
