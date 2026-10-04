import { describe, expect, it } from "vitest";
import type { Role } from "../src/generated/events";
import { nameHueOf, shownApartRole } from "../src/nameHue";

function role(id: string, position: number, hue: number | null, hoist = false): Role {
  return { id, community: "c", name: id, position, permissions: [], everyone: false, hue, hoist };
}

const roles = [role("low", 1, 30, true), role("plain", 2, null), role("high", 3, 200, true)];

describe("name hues", () => {
  it("take the highest held role that has a hue", () => {
    expect(nameHueOf({}, roles, ["low", "high"])).toBe(200);
    expect(nameHueOf({}, roles, ["low", "plain"])).toBe(30);
  });

  it("pass over a higher role without one", () => {
    expect(nameHueOf({}, roles, ["plain"])).toBeUndefined();
  });

  it("put a deployment role's hue over every community role's, and show it anywhere", () => {
    expect(nameHueOf({ nameHue: 90 }, roles, ["high"])).toBe(90);
    expect(nameHueOf({ nameHue: 90 })).toBe(90);
    expect(nameHueOf({ nameHue: 0 })).toBe(0);
  });

  it("leave a name plain with no roles at all", () => {
    expect(nameHueOf(undefined)).toBeUndefined();
    expect(nameHueOf({ nameHue: null }, roles, [])).toBeUndefined();
  });
});

describe("roles shown apart", () => {
  it("list a member under the highest they hold", () => {
    expect(shownApartRole(roles, ["low", "high"])?.id).toBe("high");
    expect(shownApartRole(roles, ["low", "plain"])?.id).toBe("low");
    expect(shownApartRole(roles, ["plain"])).toBeUndefined();
    expect(shownApartRole(roles, undefined)).toBeUndefined();
  });
});
