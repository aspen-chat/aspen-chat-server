import { describe, expect, it } from "vitest";
import { DEFAULT_BODY_PX, isAppleTouch, systemTextScale } from "./systemTextSize";

describe("systemTextScale", () => {
  it("is 1 at the default body size and grows with it", () => {
    expect(systemTextScale(DEFAULT_BODY_PX)).toBe(1);
    expect(systemTextScale(34)).toBe(2);
    expect(systemTextScale(14)).toBeCloseTo(14 / 17);
  });

  it("stays normal for a size it cannot read", () => {
    expect(systemTextScale(Number.NaN)).toBe(1);
    expect(systemTextScale(0)).toBe(1);
  });
});

describe("isAppleTouch", () => {
  it("knows an iPhone, and an iPad that names itself a Mac", () => {
    expect(isAppleTouch("Mozilla/5.0 (iPhone; CPU iPhone OS 27_0 like Mac OS X)", 5)).toBe(true);
    expect(isAppleTouch("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", 5)).toBe(true);
  });

  it("leaves a Mac and Android alone", () => {
    expect(isAppleTouch("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", 0)).toBe(false);
    expect(isAppleTouch("Mozilla/5.0 (Linux; Android 16; Pixel 7)", 5)).toBe(false);
  });
});
