import type { Category, Channel } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { layoutRows, planMove, TOP_LEVEL_KEY, type LayoutRow } from "./layoutOrder";

const category = (id: string, sortIndex: number): Category => ({
  id,
  community: "community",
  name: id,
  sortIndex,
});

const channel = (id: string, parentCategory: string | null, sortIndex: number): Channel => ({
  id,
  community: "community",
  parentCategory,
  name: id,
  ty: "text",
  sortIndex,
  parentChannel: null,
  starterMessage: null,
  replyCount: 0,
  lastReplyAt: null,
  recipients: [],
  pluginType: null,
});

/**
 * Two top-level channels, then category A with two, B with none, and C with one:
 * `top-level t1 t2 A a1 a2 B C c1`.
 */
function rows(): LayoutRow[] {
  const categories = [category("A", 0), category("B", 1), category("C", 2)];
  return layoutRows(
    [channel("t1", null, 0), channel("t2", null, 1)],
    categories,
    new Map([
      ["A", [channel("a1", "A", 0), channel("a2", "A", 1)]],
      ["B", []],
      ["C", [channel("c1", "C", 0)]],
    ]),
  );
}

const before = (key: string) => ({ key, dropPosition: "before" as const });
const after = (key: string) => ({ key, dropPosition: "after" as const });
const on = (key: string) => ({ key, dropPosition: "on" as const });

describe("layoutRows", () => {
  it("lists the top level first, then each category with its channels", () => {
    expect(rows().map((row) => row.key)).toEqual([
      TOP_LEVEL_KEY,
      "t1",
      "t2",
      "A",
      "a1",
      "a2",
      "B",
      "C",
      "c1",
    ]);
  });
});

describe("planMove for a channel", () => {
  const channels = (parentCategory: string | null, order: string[]) => ({
    kind: "channels",
    parentCategory,
    order,
  });

  it("reorders within its group", () => {
    expect(planMove(rows(), "a2", before("a1"))).toEqual(channels("A", ["a2", "a1"]));
    expect(planMove(rows(), "t1", after("t2"))).toEqual(channels(null, ["t2", "t1"]));
  });

  it("moves into another category at the place dropped", () => {
    expect(planMove(rows(), "t1", after("a1"))).toEqual(channels("A", ["a1", "t1", "a2"]));
    expect(planMove(rows(), "c1", before("a1"))).toEqual(channels("A", ["c1", "a1", "a2"]));
  });

  it("goes first in a group when dropped under its heading", () => {
    expect(planMove(rows(), "a2", after("A"))).toEqual(channels("A", ["a2", "a1"]));
    expect(planMove(rows(), "t1", after("B"))).toEqual(channels("B", ["t1"]));
    expect(planMove(rows(), "a1", after(TOP_LEVEL_KEY))).toEqual(
      channels(null, ["a1", "t1", "t2"]),
    );
  });

  it("goes last in the group above when dropped just over a category's heading", () => {
    expect(planMove(rows(), "c1", before("A"))).toEqual(channels(null, ["t1", "t2", "c1"]));
    expect(planMove(rows(), "t1", before("B"))).toEqual(channels("A", ["a1", "a2", "t1"]));
    // B is empty, so the gap over C's heading is still B's.
    expect(planMove(rows(), "t1", before("C"))).toEqual(channels("B", ["t1"]));
  });

  it("goes to the end of a group when dropped on its heading", () => {
    expect(planMove(rows(), "t1", on("A"))).toEqual(channels("A", ["a1", "a2", "t1"]));
    expect(planMove(rows(), "a1", on(TOP_LEVEL_KEY))).toEqual(channels(null, ["t1", "t2", "a1"]));
    expect(planMove(rows(), "a1", on("A"))).toEqual(channels("A", ["a2", "a1"]));
  });

  it("refuses a drop that changes nothing or lands nowhere", () => {
    expect(planMove(rows(), "a1", after("A"))).toBeNull();
    expect(planMove(rows(), "a1", before("a2"))).toBeNull();
    expect(planMove(rows(), "a1", after("a1"))).toBeNull();
    expect(planMove(rows(), "a2", on("A"))).toBeNull();
    expect(planMove(rows(), "a1", on("a2"))).toBeNull();
    expect(planMove(rows(), "a1", before(TOP_LEVEL_KEY))).toBeNull();
  });
});

describe("planMove for a category", () => {
  const categories = (order: string[]) => ({ kind: "categories", order });

  it("moves with its channels to just over another category's heading", () => {
    expect(planMove(rows(), "C", before("A"))).toEqual(categories(["C", "A", "B"]));
    expect(planMove(rows(), "A", before("C"))).toEqual(categories(["B", "A", "C"]));
  });

  it("takes the gap under the last row of the group above as that same place", () => {
    expect(planMove(rows(), "C", after("t2"))).toEqual(categories(["C", "A", "B"]));
    expect(planMove(rows(), "C", after("a2"))).toEqual(categories(["A", "C", "B"]));
    expect(planMove(rows(), "A", after("B"))).toEqual(categories(["B", "A", "C"]));
  });

  it("moves to the end of the list", () => {
    expect(planMove(rows(), "A", after("c1"))).toEqual(categories(["B", "C", "A"]));
  });

  it("refuses a place inside a group, a drop on a row, and staying put", () => {
    expect(planMove(rows(), "C", after("t1"))).toBeNull();
    expect(planMove(rows(), "C", after("A"))).toBeNull();
    expect(planMove(rows(), "C", before("a2"))).toBeNull();
    expect(planMove(rows(), "C", on("A"))).toBeNull();
    expect(planMove(rows(), "C", before(TOP_LEVEL_KEY))).toBeNull();
    expect(planMove(rows(), "A", before("a2"))).toBeNull();
    expect(planMove(rows(), "A", before("B"))).toBeNull();
    expect(planMove(rows(), "C", after("c1"))).toBeNull();
    expect(planMove(rows(), "B", before("C"))).toBeNull();
  });

  it("never moves the top level's heading", () => {
    expect(planMove(rows(), TOP_LEVEL_KEY, after("c1"))).toBeNull();
  });
});
