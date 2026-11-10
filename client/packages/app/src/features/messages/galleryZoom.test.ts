import { describe, expect, test } from "vitest";
import {
  DOUBLE_TAP_ZOOM,
  FIT,
  ZOOM_OVER_FIT,
  coasted,
  doubleTapped,
  held,
  maxScale,
  moved,
  panRange,
  rested,
  sameView,
  zoomedAbout,
  type Frame,
  type Point,
  type View,
} from "./galleryZoom";

/** A landscape photo on a phone held upright: as wide as the stage, and far shorter. */
const frame: Frame = {
  stage: { width: 400, height: 600 },
  fitted: { width: 400, height: 300 },
  natural: { width: 800, height: 600 },
};

/** Where on the screen the picture's own point `point` (from its centre, at fit) is drawn. */
const drawnAt = (view: View, point: Point): Point => ({
  x: view.x + point.x * view.scale,
  y: view.y + point.y * view.scale,
});

describe("maxScale", () => {
  test("a small original still zooms several times over", () => {
    expect(maxScale(frame)).toBe(ZOOM_OVER_FIT);
  });

  test("a large original zooms until its pixels are the screen's", () => {
    expect(maxScale({ ...frame, natural: { width: 4000, height: 3000 } })).toBe(10);
  });
});

describe("panRange", () => {
  test("a fitted picture rests only at the stage's centre", () => {
    expect(panRange(1, frame)).toEqual({ x: { min: 0, max: 0 }, y: { min: 0, max: 0 } });
  });

  test("an axis pans once the picture is larger than the stage along it", () => {
    // Twice the size it is 800 by 600: 400 wider than the stage, and exactly as tall.
    expect(panRange(2, frame)).toEqual({ x: { min: -200, max: 200 }, y: { min: 0, max: 0 } });
    expect(panRange(3, frame).y).toEqual({ min: -150, max: 150 });
  });
});

describe("zoomedAbout", () => {
  test("keeps what is under the point there", () => {
    const about = { x: 120, y: -40 };
    const from: View = { scale: 1.5, x: 30, y: 10 };
    // The picture's own point under `about` before the zoom.
    const under = { x: (about.x - from.x) / from.scale, y: (about.y - from.y) / from.scale };
    const to = zoomedAbout(from, about, 3);
    expect(to.scale).toBe(3);
    expect(drawnAt(to, under).x).toBeCloseTo(about.x);
    expect(drawnAt(to, under).y).toBeCloseTo(about.y);
  });
});

describe("moved", () => {
  test("one finger pans by as far as it travels", () => {
    const from: View = { scale: 2, x: 10, y: 0 };
    const to = moved(
      from,
      { centre: { x: 0, y: 0 }, span: 0 },
      { centre: { x: 25, y: -15 }, span: 0 },
    );
    expect(to).toEqual({ scale: 2, x: 35, y: -15 });
  });

  test("two fingers parting zoom about their middle, which carries the picture with it", () => {
    const was = { centre: { x: 100, y: 50 }, span: 80 };
    const now = { centre: { x: 110, y: 70 }, span: 160 };
    const to = moved(FIT, was, now);
    expect(to.scale).toBe(2);
    // What was under the fingers' middle is under it still.
    expect(drawnAt(to, was.centre)).toEqual(now.centre);
  });

  test("two fingers brought to one point leave a view that can be scaled back", () => {
    const to = moved(
      FIT,
      { centre: { x: 0, y: 0 }, span: 80 },
      { centre: { x: 0, y: 0 }, span: 0 },
    );
    expect(to.scale).toBeGreaterThan(0);
  });
});

describe("held", () => {
  const origin = { x: 0, y: 0 };

  test("shows a view within its limits as asked", () => {
    const asked: View = { scale: 2, x: 150, y: 0 };
    expect(held(asked, origin, frame, false)).toEqual({ view: asked, beyond: 0 });
  });

  test("gives a little past the largest scale, and past the smallest", () => {
    const over = held({ scale: 8, x: 0, y: 0 }, origin, frame, false).view.scale;
    expect(over).toBeGreaterThan(ZOOM_OVER_FIT);
    expect(over).toBeLessThan(8);
    const under = held({ scale: 0.5, x: 0, y: 0 }, origin, frame, false).view.scale;
    expect(under).toBeLessThan(1);
    expect(under).toBeGreaterThan(0.5);
  });

  test("gives a little when panned past the stage's edge", () => {
    const { view, beyond } = held({ scale: 2, x: 300, y: 0 }, origin, frame, false);
    expect(view.x).toBeGreaterThan(200);
    expect(view.x).toBeLessThan(300);
    expect(beyond).toBe(0);
  });

  test("carrying, stops at the stage's side and hands on the rest", () => {
    expect(held({ scale: 2, x: 300, y: 0 }, origin, frame, true)).toEqual({
      view: { scale: 2, x: 200, y: 0 },
      beyond: 100,
    });
    expect(held({ scale: 2, x: -260, y: 0 }, origin, frame, true).beyond).toBe(-60);
    expect(held({ scale: 2, x: 120, y: 0 }, origin, frame, true).beyond).toBe(0);
  });

  test("carrying still gives up and down, where there is no picture to draw in", () => {
    const { view } = held({ scale: 3, x: 0, y: 400 }, origin, frame, true);
    expect(view.y).toBeGreaterThan(150);
    expect(view.y).toBeLessThan(400);
  });
});

describe("rested", () => {
  test("a view within its limits rests where it is", () => {
    const asked: View = { scale: 2, x: -150, y: 0 };
    expect(rested(asked, { x: 0, y: 0 }, frame)).toEqual(asked);
  });

  test("pinched smaller than fitted, it goes back to fitted", () => {
    expect(rested({ scale: 0.6, x: 40, y: -25 }, { x: 90, y: 10 }, frame)).toEqual(FIT);
  });

  test("zoomed too far, it comes back about the fingers and stays inside the stage", () => {
    const about = { x: 100, y: 0 };
    const at = rested(zoomedAbout(FIT, about, 9), about, frame);
    expect(at.scale).toBe(ZOOM_OVER_FIT);
    expect(drawnAt(at, about).x).toBeCloseTo(about.x);
  });

  test("panned past an edge, it comes back to the edge", () => {
    expect(rested({ scale: 2, x: 500, y: 90 }, { x: 0, y: 0 }, frame)).toEqual({
      scale: 2,
      x: 200,
      y: 0,
    });
  });
});

describe("doubleTapped", () => {
  test("zooms a fitted picture in on the point tapped", () => {
    const about = { x: 50, y: 20 };
    const to = doubleTapped(FIT, about, frame);
    expect(to.scale).toBe(DOUBLE_TAP_ZOOM);
    expect(drawnAt(to, about).x).toBeCloseTo(about.x);
  });

  test("keeps the stage covered when the tap is near the edge of a picture shorter than it", () => {
    // Zoomed about a point near its foot, the picture would leave the stage's foot bare.
    const to = doubleTapped(FIT, { x: 0, y: 140 }, frame);
    expect(to.y).toBe(panRange(DOUBLE_TAP_ZOOM, frame).y.min);
  });

  test("fits a zoomed picture again", () => {
    expect(doubleTapped({ scale: 2, x: 30, y: 0 }, { x: 0, y: 0 }, frame)).toEqual(FIT);
  });
});

describe("coasted", () => {
  test("carries on the way it was thrown, slowing", () => {
    const next = coasted({ scale: 3, x: 0, y: 0 }, { x: 1, y: -0.5 }, 16, frame);
    expect(next.view.x).toBeGreaterThan(0);
    expect(next.view.y).toBeLessThan(0);
    expect(next.velocity.x).toBeLessThan(1);
    expect(next.velocity.x).toBeGreaterThan(0);
  });

  test("stops dead at an edge along that axis alone", () => {
    const next = coasted({ scale: 3, x: 395, y: 0 }, { x: 2, y: 1 }, 16, frame);
    expect(next.view.x).toBe(panRange(3, frame).x.max);
    expect(next.velocity.x).toBe(0);
    expect(next.velocity.y).toBeGreaterThan(0);
  });
});

describe("sameView", () => {
  test("takes views no eye could tell apart as one, and any others as two", () => {
    expect(sameView(FIT, { scale: 1.0000001, x: 0.01, y: -0.01 })).toBe(true);
    expect(sameView(FIT, { scale: 1.01, x: 0, y: 0 })).toBe(false);
    expect(sameView(FIT, { scale: 1, x: 0, y: 1 })).toBe(false);
  });
});
