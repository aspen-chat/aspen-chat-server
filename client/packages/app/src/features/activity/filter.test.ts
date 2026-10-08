import { describe, expect, it } from "vitest";
import { NO_COMMUNITY, hiddenKey, readFilter, shows } from "./filter";

const hide = (...keys: string[]) => new Set(keys);

describe("the activity feed's filter", () => {
  it("asks for everything while nothing is hidden", () => {
    expect(readFilter(hide(), null, ["a", "b"])).toEqual({ dms: true });
  });

  it("leaves a hidden deployment out whole", () => {
    const hidden = hide(hiddenKey({ kind: "deployment", domain: "far.example" }));
    expect(readFilter(hidden, "far.example", ["a"])).toBeNull();
    expect(readFilter(hidden, null, ["a"])).toEqual({ dms: true });
    expect(shows(hidden, "far.example", null)).toBe(false);
  });

  it("names the communities that show once one is hidden", () => {
    const hidden = hide(hiddenKey({ kind: "community", domain: null, community: "a" }));
    expect(readFilter(hidden, null, ["a", "b"])).toEqual({ dms: true, communities: ["b"] });
    expect(shows(hidden, null, "a")).toBe(false);
    expect(shows(hidden, null, "b")).toBe(true);
  });

  it("asks for DMs alone with a community no one belongs to", () => {
    const hidden = hide(hiddenKey({ kind: "community", domain: null, community: "a" }));
    expect(readFilter(hidden, null, ["a"])).toEqual({ dms: true, communities: [NO_COMMUNITY] });
    const none = hide(...[...hidden, hiddenKey({ kind: "dms", domain: null })]);
    expect(readFilter(none, null, ["a"])).toBeNull();
    expect(shows(none, null, null)).toBe(false);
  });

  it("keeps a deployment's parts apart from another's", () => {
    const hidden = hide(hiddenKey({ kind: "dms", domain: "far.example" }));
    expect(readFilter(hidden, null, [])).toEqual({ dms: true });
    expect(readFilter(hidden, "far.example", ["a"])).toEqual({ dms: false });
  });
});
