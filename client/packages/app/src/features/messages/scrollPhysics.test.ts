import { describe, expect, it } from "vitest";
import {
  COASTING_STOPS,
  SPRING_MS,
  VelocityTracker,
  bounds,
  clamp,
  coast,
  rubberBand,
  shown,
  spring,
  thumb,
} from "./scrollPhysics";

describe("bounds", () => {
  it("runs from the top to the bottom of content taller than the viewport", () => {
    expect(bounds(1000, 400)).toEqual({ min: 0, max: 600 });
  });

  it("is one offset, showing content that fits at the bottom", () => {
    expect(bounds(300, 400)).toEqual({ min: -100, max: -100 });
    expect(clamp(50, bounds(300, 400))).toBe(-100);
  });
});

describe("rubberBand", () => {
  it("shows a pull past an end, ever less of it, and never more than part of the viewport", () => {
    expect(rubberBand(0, 800)).toBe(0);
    const small = rubberBand(50, 800);
    const large = rubberBand(2000, 800);
    expect(small).toBeGreaterThan(0);
    expect(small).toBeLessThan(50);
    expect(large).toBeGreaterThan(small);
    expect(large).toBeLessThan(800 * 0.55);
    expect(rubberBand(-50, 800)).toBe(-small);
  });

  it("is applied past either end and nowhere else", () => {
    const range = { min: 0, max: 600 };
    expect(shown(300, range, 800)).toBe(300);
    expect(shown(-100, range, 800)).toBe(rubberBand(-100, 800));
    expect(shown(700, range, 800)).toBe(600 + rubberBand(100, 800));
  });
});

describe("coast", () => {
  it("slows down and travels less and less", () => {
    const first = coast(2, 100);
    const second = coast(first.velocity, 100);
    expect(first.velocity).toBeLessThan(2);
    expect(first.velocity).toBeGreaterThan(second.velocity);
    expect(first.travelled).toBeGreaterThan(second.travelled);
    expect(first.travelled).toBeGreaterThan(0);
  });

  it("comes to a stop in a few seconds from a fast flick", () => {
    let velocity = 5;
    let elapsed = 0;
    while (Math.abs(velocity) >= COASTING_STOPS && elapsed < 10_000) {
      velocity = coast(velocity, 16).velocity;
      elapsed += 16;
    }
    expect(elapsed).toBeGreaterThan(1000);
    expect(elapsed).toBeLessThan(5000);
  });
});

describe("spring", () => {
  it("eases from one offset to the other without overshooting", () => {
    expect(spring(100, 0, 0)).toBe(100);
    const mid = spring(100, 0, SPRING_MS / 2);
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(100);
    expect(spring(100, 0, SPRING_MS)).toBe(0);
    expect(spring(100, 0, SPRING_MS * 2)).toBe(0);
  });
});

describe("VelocityTracker", () => {
  it("measures a finger's speed over its last stretch", () => {
    const tracker = new VelocityTracker();
    tracker.add(0, 0);
    tracker.add(50, 50);
    tracker.add(100, 100);
    expect(tracker.velocity(100)).toBe(1);
  });

  it("forgets what is older than the window", () => {
    const tracker = new VelocityTracker();
    tracker.add(0, 0);
    tracker.add(1000, 50);
    tracker.add(1100, 200);
    tracker.add(1200, 250);
    expect(tracker.velocity(250)).toBe(2);
  });

  it("is zero for a finger that rested before lifting", () => {
    const tracker = new VelocityTracker();
    tracker.add(0, 0);
    tracker.add(100, 50);
    expect(tracker.velocity(300)).toBe(0);
  });
});

describe("thumb", () => {
  const sizes = { viewportHeight: 400, contentHeight: 1600, trackHeight: 392, minHeight: 24 };

  it("is as tall as the share of the content in view, and runs the track's length", () => {
    const range = bounds(1600, 400);
    expect(thumb(range.min, range, sizes)).toEqual({ height: 98, top: 0, shown: true });
    expect(thumb(range.max, range, sizes)).toEqual({ height: 98, top: 294, shown: true });
  });

  it("is never shorter than its least height", () => {
    const range = bounds(160_000, 400);
    expect(thumb(0, range, { ...sizes, contentHeight: 160_000 }).height).toBe(24);
  });

  it("fills the track, unshown, for content that fits", () => {
    const range = bounds(300, 400);
    expect(thumb(range.min, range, { ...sizes, contentHeight: 300 })).toEqual({
      height: 392,
      top: 0,
      shown: false,
    });
  });
});
