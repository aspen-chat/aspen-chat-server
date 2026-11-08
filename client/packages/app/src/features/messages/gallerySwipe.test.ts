import { describe, expect, test } from "vitest";
import { FLICK, swipeOutcome } from "./gallerySwipe";

const both = { hasPrevious: true, hasNext: true };
const WIDTH = 400;

describe("swipeOutcome", () => {
  test("a drag past halfway toward the start lands on the next picture", () => {
    expect(swipeOutcome(-201, 0, WIDTH, both)).toBe(1);
  });

  test("a drag past halfway toward the end lands on the previous picture", () => {
    expect(swipeOutcome(201, 0, WIDTH, both)).toBe(-1);
  });

  test("a drag that comes to rest short of halfway springs back", () => {
    expect(swipeOutcome(-199, 0, WIDTH, both)).toBe(0);
    expect(swipeOutcome(199, 0, WIDTH, both)).toBe(0);
  });

  test("a throw lands the way it was thrown, however short the drag", () => {
    expect(swipeOutcome(-20, -FLICK - 0.01, WIDTH, both)).toBe(1);
    expect(swipeOutcome(20, FLICK + 0.01, WIDTH, both)).toBe(-1);
  });

  test("a throw back the way the finger came overrides the distance dragged", () => {
    expect(swipeOutcome(-300, FLICK + 0.01, WIDTH, both)).toBe(-1);
  });

  test("a swipe toward a picture that is not there lands where it was", () => {
    expect(swipeOutcome(-300, -1, WIDTH, { hasPrevious: true, hasNext: false })).toBe(0);
    expect(swipeOutcome(300, 1, WIDTH, { hasPrevious: false, hasNext: true })).toBe(0);
  });
});
