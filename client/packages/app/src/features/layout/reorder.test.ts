import { describe, expect, it } from "vitest";
import { insertIds, reorderIds } from "./reorder";

describe("insertIds", () => {
  it("places arriving items before or after the target, or at the end", () => {
    expect(insertIds(["a", "b"], ["x", "y"], { key: "b", dropPosition: "before" })).toEqual([
      "a",
      "x",
      "y",
      "b",
    ]);
    expect(insertIds(["a", "b"], ["x"], { key: "a", dropPosition: "on" })).toEqual(["a", "x", "b"]);
    expect(insertIds(["a", "b"], ["x"], { key: "zz", dropPosition: "after" })).toEqual([
      "a",
      "b",
      "x",
    ]);
    expect(insertIds([], ["x"], { key: "zz", dropPosition: "after" })).toEqual(["x"]);
  });
});

describe("reorderIds", () => {
  const ids = ["a", "b", "c", "d"];
  it("moves an item before or after the target", () => {
    expect(reorderIds(ids, new Set(["d"]), { key: "a", dropPosition: "before" })).toEqual([
      "d",
      "a",
      "b",
      "c",
    ]);
    expect(reorderIds(ids, new Set(["a"]), { key: "c", dropPosition: "after" })).toEqual([
      "b",
      "c",
      "a",
      "d",
    ]);
    expect(reorderIds(ids, new Set(["a"]), { key: "c", dropPosition: "on" })).toEqual([
      "b",
      "c",
      "a",
      "d",
    ]);
  });

  it("keeps several moved items together and in order", () => {
    expect(reorderIds(ids, new Set(["a", "c"]), { key: "d", dropPosition: "after" })).toEqual([
      "b",
      "d",
      "a",
      "c",
    ]);
  });

  it("leaves the order alone for a drop on itself or an unknown target", () => {
    expect(reorderIds(ids, new Set(["b"]), { key: "b", dropPosition: "after" })).toEqual(ids);
    expect(reorderIds(ids, new Set(["b"]), { key: "zz", dropPosition: "after" })).toEqual(ids);
  });
});
