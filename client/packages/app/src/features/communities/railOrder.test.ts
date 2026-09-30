import type { RailFolder } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import {
  arrangeRail,
  folderKey,
  moveInRail,
  railKey,
  railSequence,
  ungroupFolder,
  updateFolder,
  type RailLayout,
  type RailUnit,
} from "./railOrder";

const home = (communityId: string) => ({ domain: null, communityId });
const at = (domain: string, communityId: string) => ({ domain, communityId });
type Place = ReturnType<typeof home> | ReturnType<typeof at>;

const folder = (id: string, members: string[], open = false): RailFolder => ({
  id,
  name: "",
  color: "accent",
  open,
  members,
});

/** The rail as a short picture: communities by key, a folder as `[a b]`, open as `[a b]+`. */
function shape(units: readonly RailUnit<Place>[]): string {
  return units
    .map((unit) =>
      unit.kind === "community"
        ? railKey(unit.entry)
        : `[${unit.entries.map(railKey).join(" ")}]${unit.folder.open ? "+" : ""}`,
    )
    .join(" ");
}

const entries = ["a", "b", "c", "d", "e"].map(home);
const newId = () => "new";
/** Applies a drop and shows the rail it leaves. */
function after(
  layout: RailLayout,
  dragged: string,
  key: string,
  position: "before" | "after" | "on",
) {
  const moved = moveInRail(arrangeRail(entries, layout), dragged, { key, position }, newId);
  return moved === null ? null : shape(arrangeRail(entries, moved));
}

describe("arrangeRail", () => {
  it("keys home communities by id and others by domain and id", () => {
    expect(railKey(home("a"))).toBe("a");
    expect(railKey(at("b.example:8443", "a"))).toBe("b.example:8443/a");
  });

  it("follows the saved order across deployments, then adds the rest at the end", () => {
    const places = [home("h1"), home("h2"), at("b.example", "f1"), at("b.example", "f2")];
    const arranged = arrangeRail(places, {
      order: ["b.example/f2", "h2", "gone", "h1"],
      folders: [],
    });
    expect(railSequence(arranged)).toEqual(["b.example/f2", "h2", "h1", "b.example/f1"]);
  });

  it("with no saved order keeps each deployment's own, home first", () => {
    const places = [home("h1"), at("b.example", "f1"), home("h2")];
    expect(railSequence(arrangeRail(places, { order: [], folders: [] }))).toEqual([
      "h1",
      "b.example/f1",
      "h2",
    ]);
  });

  it("stands a folder where the order names it, holding its communities in its own order", () => {
    const layout = { order: ["e", folderKey("f"), "a"], folders: [folder("f", ["d", "b"])] };
    expect(shape(arrangeRail(entries, layout))).toBe("e [d b] a c");
  });

  it("leaves out communities no longer belonged to, and folders left with none", () => {
    const layout = {
      order: [folderKey("f"), folderKey("g")],
      folders: [folder("f", ["gone", "b"]), folder("g", ["gone too"])],
    };
    expect(shape(arrangeRail(entries, layout))).toBe("[b] a c d e");
  });

  it("gives a community two folders claim to the first, and shows an unordered folder after the rest", () => {
    const layout = { order: ["a"], folders: [folder("f", ["b", "c"]), folder("g", ["c", "d"])] };
    expect(shape(arrangeRail(entries, layout))).toBe("a [b c] [d] e");
  });
});

describe("moveInRail", () => {
  const flat: RailLayout = { order: ["a", "b", "c", "d", "e"], folders: [] };
  const closed: RailLayout = {
    order: ["a", folderKey("f"), "d", "e"],
    folders: [folder("f", ["b", "c"])],
  };
  const open: RailLayout = {
    order: ["a", folderKey("f"), "d", "e"],
    folders: [folder("f", ["b", "c"], true)],
  };

  it("moves a community among communities", () => {
    expect(after(flat, "a", "c", "after")).toBe("b c a d e");
    expect(after(flat, "e", "a", "before")).toBe("e a b c d");
  });

  it("makes a folder of a community dropped on another, where the other stood", () => {
    expect(after(flat, "d", "b", "on")).toBe("a [b d] c e");
  });

  it("adds a community dropped on a folder, or on one of its communities, to the folder", () => {
    expect(after(closed, "e", folderKey("f"), "on")).toBe("a [b c e] d");
    expect(after(open, "e", "b", "on")).toBe("a [b e c]+ d");
  });

  it("places a community among an open folder's own, and after its last takes it out", () => {
    expect(after(open, "a", "c", "before")).toBe("[b a c]+ d e");
    expect(after(open, "e", folderKey("f"), "after")).toBe("a [e b c]+ d");
    expect(after(open, "b", "c", "after")).toBe("a c b d e");
  });

  it("puts a community beside a closed folder rather than in it", () => {
    expect(after(closed, "e", folderKey("f"), "after")).toBe("a [b c] e d");
    expect(after(closed, "e", folderKey("f"), "before")).toBe("a e [b c] d");
  });

  it("undoes a folder that a move leaves with one community, which takes its place", () => {
    expect(after(closed, "b", "e", "after")).toBe("a c d e b");
  });

  it("moves a folder whole, never onto anything, and beside the folder a member is in", () => {
    expect(after(closed, folderKey("f"), "e", "after")).toBe("a d e [b c]");
    expect(after(closed, folderKey("f"), "a", "on")).toBeNull();
    const two = {
      order: [folderKey("f"), folderKey("g"), "e"],
      folders: [folder("f", ["a", "b"]), folder("g", ["c", "d"], true)],
    };
    expect(after(two, folderKey("f"), "c", "before")).toBe("[c d]+ [a b] e");
  });

  it("ignores a drop onto itself", () => {
    expect(after(flat, "a", "a", "on")).toBeNull();
  });

  it("keeps a folder's name, colour, and state through moves", () => {
    const named: RailLayout = {
      order: ["a", folderKey("f")],
      folders: [{ ...folder("f", ["b", "c"], true), name: "Games", color: "rose" }],
    };
    const moved = moveInRail(arrangeRail(entries, named), "d", { key: "b", position: "on" }, newId);
    expect(moved?.folders).toEqual([
      { id: "f", name: "Games", color: "rose", open: true, members: ["b", "d", "c"] },
    ]);
  });
});

describe("updateFolder and ungroupFolder", () => {
  const layout: RailLayout = {
    order: ["a", folderKey("f"), "d"],
    folders: [folder("f", ["b", "c"])],
  };

  it("changes one folder and nothing else", () => {
    const changed = updateFolder(arrangeRail(entries, layout), "f", { open: true, name: "Work" });
    expect(changed.folders).toEqual([{ ...folder("f", ["b", "c"], true), name: "Work" }]);
    expect(shape(arrangeRail(entries, changed))).toBe("a [b c]+ d e");
  });

  it("stands a folder's communities where it stood", () => {
    const undone = ungroupFolder(arrangeRail(entries, layout), "f");
    expect(undone.folders).toEqual([]);
    expect(shape(arrangeRail(entries, undone))).toBe("a b c d e");
  });
});
