import { describe, expect, it } from "vitest";
import { MAX_ZOOM, MIN_ZOOM, ZOOM_STEPS, steppedZoom, zoomFactor } from "./zoom";

describe("steppedZoom", () => {
  it("steps to the neighbouring factors", () => {
    expect(steppedZoom(1, 1)).toBe(1.1);
    expect(steppedZoom(1, -1)).toBe(0.9);
    expect(steppedZoom(1.25, 1)).toBe(1.5);
    expect(steppedZoom(0.67, -1)).toBe(0.5);
  });

  it("goes back to normal", () => {
    expect(steppedZoom(2.5, 0)).toBe(1);
  });

  it("goes from between two steps to the nearer one in that direction", () => {
    expect(steppedZoom(1.3, 1)).toBe(1.5);
    expect(steppedZoom(1.3, -1)).toBe(1.25);
  });

  it("stays within the bounds", () => {
    expect(steppedZoom(MAX_ZOOM, 1)).toBe(MAX_ZOOM);
    expect(steppedZoom(MIN_ZOOM, -1)).toBe(MIN_ZOOM);
  });

  it("starts and ends on the bounds", () => {
    expect(ZOOM_STEPS[0]).toBe(MIN_ZOOM);
    expect(ZOOM_STEPS.at(-1)).toBe(MAX_ZOOM);
  });
});

describe("zoomFactor", () => {
  it("keeps factors within the bounds and refuses anything else", () => {
    expect(zoomFactor(1.25)).toBe(1.25);
    expect(zoomFactor(10)).toBe(MAX_ZOOM);
    expect(zoomFactor(0)).toBe(MIN_ZOOM);
    expect(zoomFactor(Number.NaN)).toBeUndefined();
    expect(zoomFactor("2")).toBeUndefined();
  });
});
