/**
 * The arithmetic of zooming a picture in the gallery: how fingers' travel becomes a view of
 * it, how far it may be zoomed and panned, and where it comes to rest. Pure functions over
 * numbers, so `useGalleryZoom` follows the fingers and the gallery only draws the view.
 *
 * Points are in CSS pixels from the stage's centre, where a fitted picture's centre is, `x`
 * growing to the right and `y` downwards.
 */
import { clamp, coast, rubberBand, shown } from "@/features/messages/scrollPhysics";

export interface Point {
  readonly x: number;
  readonly y: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

/**
 * How a picture is drawn over its fitted place: `scale` times its fitted size, its centre `x`
 * and `y` from the stage's.
 */
export interface View {
  readonly scale: number;
  readonly x: number;
  readonly y: number;
}

/** A picture fitted to the stage, as the gallery shows one nobody has zoomed. */
export const FIT: View = { scale: 1, x: 0, y: 0 };

/** What a picture is zoomed within: the stage, the picture as fitted to it, and its original. */
export interface Frame {
  readonly stage: Size;
  readonly fitted: Size;
  readonly natural: Size;
}

/** Where fingers hold a picture: their middle, and how far apart they are (0 for one finger). */
export interface Grip {
  readonly centre: Point;
  readonly span: number;
}

/** How many times its fitted size any picture may be zoomed to. */
export const ZOOM_OVER_FIT = 4;
/** How many times its fitted size a double-tap zooms a picture to, where it may go that far. */
export const DOUBLE_TAP_ZOOM = 2.5;
/**
 * The least scale fingers can pinch a view to. Two fingers on one point span nothing, and from
 * a scale of zero no later span could scale the view back.
 */
const LEAST_SCALE = 0.05;

/**
 * The furthest a picture zooms: `ZOOM_OVER_FIT` times its fitted size, or further where its
 * original has more to show, to one of its pixels per CSS pixel.
 */
export function maxScale(frame: Frame): number {
  const original = frame.fitted.width > 0 ? frame.natural.width / frame.fitted.width : 0;
  return Math.max(ZOOM_OVER_FIT, original);
}

/**
 * Where a picture's centre may rest at `scale`, along each axis: anywhere that keeps the
 * stage covered where the picture is larger than it, and at the stage's centre where it is
 * not.
 */
export function panRange(
  scale: number,
  frame: Frame,
): { x: { min: number; max: number }; y: { min: number; max: number } } {
  const reach = (fitted: number, stage: number) => {
    const most = Math.max(0, (fitted * scale - stage) / 2);
    // Subtracted, not negated: a picture with no room to pan rests at 0, never at -0.
    return { min: 0 - most, max: most };
  };
  return {
    x: reach(frame.fitted.width, frame.stage.width),
    y: reach(frame.fitted.height, frame.stage.height),
  };
}

/** `view` at `scale`, with whatever of the picture is at `about` still there. */
export function zoomedAbout(view: View, about: Point, scale: number): View {
  const ratio = scale / view.scale;
  return {
    scale,
    x: about.x - (about.x - view.x) * ratio,
    y: about.y - (about.y - view.y) * ratio,
  };
}

/**
 * The view fingers ask for when they held `from` at `was` and are now at `now`: what was under
 * their middle stays under it, and the picture grows as they part. It may be zoomed or panned
 * further than a picture goes; `held` and `rested` say what is shown of it.
 */
export function moved(from: View, was: Grip, now: Grip): View {
  const scale = Math.max(LEAST_SCALE, from.scale * (was.span > 0 ? now.span / was.span : 1));
  const at = zoomedAbout(from, was.centre, scale);
  return {
    scale,
    x: at.x + now.centre.x - was.centre.x,
    y: at.y + now.centre.y - was.centre.y,
  };
}

/** A scale past either limit gives ever less, as a list pulled past its end does. */
function resistedScale(scale: number, max: number): number {
  if (scale > max) {
    return max * (1 + rubberBand(scale / max - 1, 1));
  }
  if (scale < 1) {
    return 1 / (1 + rubberBand(1 / scale - 1, 1));
  }
  return scale;
}

/**
 * What is shown while fingers whose middle is at `about` ask for `asked`: zoomed or panned
 * past a limit, the picture gives a little. With `carry`, a picture panned past the stage's
 * side stops at it instead, and `beyond` is how far past it was asked to go, for the gallery
 * to draw the next picture in by.
 */
export function held(
  asked: View,
  about: Point,
  frame: Frame,
  carry: boolean,
): { view: View; beyond: number } {
  const at = zoomedAbout(asked, about, resistedScale(asked.scale, maxScale(frame)));
  const range = panRange(at.scale, frame);
  const x = carry ? clamp(at.x, range.x) : shown(at.x, range.x, frame.stage.width);
  return {
    view: { scale: at.scale, x, y: shown(at.y, range.y, frame.stage.height) },
    beyond: carry ? at.x - x : 0,
  };
}

/**
 * Where the picture rests once fingers whose middle was at `about` let go of `asked`: within
 * the limits of its scale, about that point, and of its panning.
 */
export function rested(asked: View, about: Point, frame: Frame): View {
  const at = zoomedAbout(asked, about, clamp(asked.scale, { min: 1, max: maxScale(frame) }));
  const range = panRange(at.scale, frame);
  return { scale: at.scale, x: clamp(at.x, range.x), y: clamp(at.y, range.y) };
}

/**
 * Where a double-tap at `about` takes `view`: a fitted picture zooms in on the point tapped,
 * and a zoomed one goes back to fitted.
 */
export function doubleTapped(view: View, about: Point, frame: Frame): View {
  if (view.scale > 1) {
    return FIT;
  }
  const scale = Math.min(DOUBLE_TAP_ZOOM, maxScale(frame));
  return rested(zoomedAbout(FIT, about, scale), about, frame);
}

/**
 * A picture thrown at `velocity` (pixels per millisecond along each axis), `elapsedMs` later:
 * it slows as a thrown list does and stops dead at the edge of where it may rest.
 */
export function coasted(
  view: View,
  velocity: Point,
  elapsedMs: number,
  frame: Frame,
): { view: View; velocity: Point } {
  const range = panRange(view.scale, frame);
  const along = (position: number, speed: number, limits: { min: number; max: number }) => {
    const next = coast(speed, elapsedMs);
    const reached = position + next.travelled;
    const kept = clamp(reached, limits);
    return { position: kept, speed: kept === reached ? next.velocity : 0 };
  };
  const x = along(view.x, velocity.x, range.x);
  const y = along(view.y, velocity.y, range.y);
  return {
    view: { scale: view.scale, x: x.position, y: y.position },
    velocity: { x: x.speed, y: y.speed },
  };
}

/** The least two views differ by, in scale and in CSS pixels, for the difference to show. */
const SCALE_SHOWS = 0.001;
const PIXELS_SHOW = 0.5;

/**
 * Whether two views are drawn alike, nearer than the eye can tell. Nothing eases between two
 * such: a transition between values the browser holds as equal never runs, and so never ends.
 */
export function sameView(a: View, b: View): boolean {
  return (
    Math.abs(a.scale - b.scale) < SCALE_SHOWS &&
    Math.abs(a.x - b.x) < PIXELS_SHOW &&
    Math.abs(a.y - b.y) < PIXELS_SHOW
  );
}
