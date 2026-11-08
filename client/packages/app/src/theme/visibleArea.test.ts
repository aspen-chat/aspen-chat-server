import { describe, expect, it } from "vitest";
import { keyboardOver, visibleInsets } from "./visibleArea";

describe("visibleInsets", () => {
  it("is nothing when the whole page is seen", () => {
    expect(visibleInsets(844, { height: 844, offsetTop: 0, scale: 1 })).toEqual({
      top: 0,
      bottom: 0,
    });
  });

  it("ends above a keyboard drawn over the page", () => {
    expect(visibleInsets(844, { height: 508, offsetTop: 0, scale: 1 })).toEqual({
      top: 0,
      bottom: 336,
    });
  });

  it("follows the page scrolled to the focused field", () => {
    expect(visibleInsets(844, { height: 508, offsetTop: 120.4, scale: 1 })).toEqual({
      top: 120,
      bottom: 216,
    });
  });

  it("is nothing while the page is pinched in", () => {
    expect(visibleInsets(844, { height: 300, offsetTop: 200, scale: 2.5 })).toEqual({
      top: 0,
      bottom: 0,
    });
  });
});

describe("keyboardOver", () => {
  it("is what a keyboard took off the page's height", () => {
    expect(keyboardOver(874, 538, 0)).toBe(336);
  });

  it("is what a keyboard drawn over the page covers", () => {
    expect(keyboardOver(844, 844, 336)).toBe(336);
  });

  it("is nothing for a bar of suggestions alone", () => {
    expect(keyboardOver(874, 819, 0)).toBeNull();
  });
});
