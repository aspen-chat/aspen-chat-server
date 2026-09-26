import { describe, expect, it } from "vitest";
import {
  boundingSquare,
  clampCircle,
  fitScale,
  ICON_MAX_SIZE,
  iconSize,
  initialCircle,
  maxRadius,
  MIN_RADIUS,
} from "./crop";

describe("crop geometry", () => {
  it("starts with the largest centred circle", () => {
    expect(initialCircle(400, 300)).toEqual({ x: 200, y: 150, radius: 150 });
    expect(maxRadius(20, 20)).toBe(MIN_RADIUS);
  });

  it("keeps the circle inside the image, radius first", () => {
    expect(clampCircle({ x: 0, y: 0, radius: 50 }, 400, 300)).toEqual({ x: 50, y: 50, radius: 50 });
    expect(clampCircle({ x: 1000, y: 1000, radius: 50 }, 400, 300)).toEqual({
      x: 350,
      y: 250,
      radius: 50,
    });
    expect(clampCircle({ x: 200, y: 150, radius: 999 }, 400, 300)).toEqual({
      x: 200,
      y: 150,
      radius: 150,
    });
    expect(clampCircle({ x: 10, y: 10, radius: 1 }, 400, 300).radius).toBe(MIN_RADIUS);
  });

  it("crops to the circle's square and caps the icon size", () => {
    expect(boundingSquare({ x: 100, y: 80, radius: 50 })).toEqual({ x: 50, y: 30, size: 100 });
    expect(iconSize({ x: 0, y: 0, radius: 50 })).toBe(100);
    expect(iconSize({ x: 0, y: 0, radius: 1000 })).toBe(ICON_MAX_SIZE);
  });

  it("scales the preview down to fit but never up", () => {
    expect(fitScale(800, 600, 400, 400)).toBe(0.5);
    expect(fitScale(100, 100, 400, 400)).toBe(1);
  });
});
