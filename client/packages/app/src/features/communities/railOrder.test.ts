import { describe, expect, it } from "vitest";
import { arrangeRail, railKey } from "./railOrder";

const home = (communityId: string) => ({ domain: null, communityId });
const at = (domain: string, communityId: string) => ({ domain, communityId });

describe("arrangeRail", () => {
  it("keys home communities by id and others by domain and id", () => {
    expect(railKey(home("a"))).toBe("a");
    expect(railKey(at("b.example:8443", "a"))).toBe("b.example:8443/a");
  });

  it("follows the saved order across deployments, then adds the rest at the end", () => {
    const entries = [home("h1"), home("h2"), at("b.example", "f1"), at("b.example", "f2")];
    const arranged = arrangeRail(entries, ["b.example/f2", "h2", "gone", "h1"]);
    expect(arranged.map(railKey)).toEqual(["b.example/f2", "h2", "h1", "b.example/f1"]);
  });

  it("with no saved order keeps each deployment's own, home first", () => {
    const entries = [home("h1"), at("b.example", "f1"), home("h2")];
    expect(arrangeRail(entries, []).map(railKey)).toEqual(["h1", "b.example/f1", "h2"]);
  });
});
