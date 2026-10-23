import { describe, expect, it } from "vitest";
import { normalizeUserCode } from "./userCode";

describe("normalizeUserCode", () => {
  it("ignores case, hyphens, and whitespace", () => {
    expect(normalizeUserCode("bcdf-ghjk")).toBe("BCDFGHJK");
    expect(normalizeUserCode(" BcDf GhJk\n")).toBe("BCDFGHJK");
    expect(normalizeUserCode("B-C-D-F-G-H-J-K")).toBe("BCDFGHJK");
  });

  it("reads full-width letters and hyphens as an input method types them", () => {
    expect(normalizeUserCode("ＢCDF－GHJK")).toBe("BCDFGHJK");
  });

  it("refuses what cannot be a code", () => {
    expect(normalizeUserCode("BCDF-GHJ")).toBeNull();
    expect(normalizeUserCode("BCDF-GHJKL")).toBeNull();
    expect(normalizeUserCode("ABCD-FGHJ")).toBeNull();
    expect(normalizeUserCode("YBCD-FGHJ")).toBeNull();
    expect(normalizeUserCode("0BCD-FGHJ")).toBeNull();
    expect(normalizeUserCode("ΒCDF-GHJK")).toBeNull();
  });
});
