import type { OverrideGrant } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { currentPreset, presetOverrides, selects } from "./accessPresets";

const EVERYONE = "everyone";
const everyoneDenied = (...deny: OverrideGrant["deny"]): OverrideGrant => ({
  role: EVERYONE,
  allow: [],
  deny,
});
const allowed = (role: string, ...allow: OverrideGrant["allow"]): OverrideGrant => ({
  role,
  allow,
  deny: [],
});

describe("currentPreset", () => {
  it("reads back each preset's own overrides, in any order", () => {
    for (const preset of ["private", "readOnly"] as const) {
      const overrides = presetOverrides(preset, new Set(["mods", "staff"]), EVERYONE);
      expect(currentPreset(overrides, EVERYONE)).toEqual({ preset, roles: ["mods", "staff"] });
      const shuffled = overrides
        .map((o) => ({ ...o, allow: [...o.allow].reverse(), deny: [...o.deny].reverse() }))
        .reverse();
      expect(currentPreset(shuffled, EVERYONE)).toEqual({ preset, roles: ["staff", "mods"] });
    }
    expect(currentPreset(presetOverrides("private", new Set(), EVERYONE), EVERYONE)).toEqual({
      preset: "private",
      roles: [],
    });
  });

  it("is open to everyone with no overrides, or only empty ones", () => {
    expect(currentPreset([], EVERYONE)).toEqual({ preset: "everyone", roles: [] });
    expect(currentPreset([everyoneDenied(), allowed("mods")], EVERYONE)).toEqual({
      preset: "everyone",
      roles: [],
    });
  });

  it("leaves an empty override out of a preset", () => {
    expect(
      currentPreset(
        [everyoneDenied("viewChannel"), allowed("mods", "viewChannel"), allowed("muted")],
        EVERYONE,
      ),
    ).toEqual({ preset: "private", roles: ["mods"] });
  });

  it("is custom when an override holds more than the preset, which applying would take away", () => {
    const custom = { preset: "custom", roles: [] };
    // A role allowed something besides, and a role denied something.
    expect(
      currentPreset(
        [everyoneDenied("viewChannel"), allowed("mods", "viewChannel", "manageMessages")],
        EVERYONE,
      ),
    ).toEqual(custom);
    expect(
      currentPreset(
        [
          everyoneDenied("viewChannel"),
          allowed("mods", "viewChannel"),
          { role: "muted", allow: [], deny: ["sendMessages"] },
        ],
        EVERYONE,
      ),
    ).toEqual(custom);
    // Everyone denied more than the preset, or allowed something too.
    expect(currentPreset([everyoneDenied("viewChannel", "sendMessages")], EVERYONE)).toEqual(
      custom,
    );
    expect(
      currentPreset([{ role: EVERYONE, allow: ["createPolls"], deny: ["viewChannel"] }], EVERYONE),
    ).toEqual(custom);
  });

  it("is custom when everyone is denied only part of what read-only withholds", () => {
    expect(currentPreset([everyoneDenied("sendMessages")], EVERYONE)).toEqual({
      preset: "custom",
      roles: [],
    });
    expect(
      currentPreset(
        [
          everyoneDenied("sendMessages", "sendInThreads", "startThreads", "createPolls"),
          allowed("mods", "sendMessages"),
        ],
        EVERYONE,
      ),
    ).toEqual({ preset: "custom", roles: [] });
  });

  it("is custom when only roles other than everyone's hold overrides", () => {
    expect(currentPreset([allowed("mods", "viewChannel")], EVERYONE)).toEqual({
      preset: "custom",
      roles: [],
    });
  });
});

describe("selects", () => {
  it("holds for the same preset and roles in any order", () => {
    expect(selects("private", new Set(["b", "a"]), { preset: "private", roles: ["a", "b"] })).toBe(
      true,
    );
  });

  it("fails on another preset, a role more, or a role fewer", () => {
    const current = { preset: "private", roles: ["a"] } as const;
    expect(selects("readOnly", new Set(["a"]), current)).toBe(false);
    expect(selects("private", new Set(["a", "b"]), current)).toBe(false);
    expect(selects("private", new Set(), current)).toBe(false);
  });
});
