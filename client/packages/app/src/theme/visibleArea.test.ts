import { describe, expect, it } from "vitest";
import { visibleInsets } from "./visibleArea";

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
