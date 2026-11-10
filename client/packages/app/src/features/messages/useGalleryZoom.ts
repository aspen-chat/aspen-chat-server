import { useEffect, useRef, type RefObject } from "react";
import {
  coasted,
  doubleTapped,
  held,
  moved,
  rested,
  sameView,
  type Frame,
  type Grip,
  type Point,
  type View,
} from "@/features/messages/galleryZoom";
import { COASTING_STOPS, MAX_VELOCITY, VelocityTracker } from "@/features/messages/scrollPhysics";

/** How far a finger moves before it is panning the picture rather than tapping it, in CSS pixels. */
const SLOP_PX = 10;
/** The longest a tap's finger stays down, and the longest between a double-tap's two, in milliseconds. */
const TAP_MS = 300;
/** How far apart a double-tap's two taps may land, in CSS pixels. */
const DOUBLE_TAP_PX = 40;

/**
 * How a view is to be drawn: `held` exactly there, under fingers or coasting after them;
 * `easing` toward it from where the picture is; or `resting` there, with nothing more to come.
 */
export type ZoomPhase = "held" | "easing" | "resting";

export interface ZoomHandlers {
  onView: (view: View, phase: ZoomPhase) => void;
  /**
   * Whether one finger panning the picture past the stage's side carries the gallery along
   * with it (`onCarry`), drawing the next picture in. Without it the picture gives a little
   * there and springs back, as it does up and down.
   */
  carries: boolean;
  /** Whether a picture thrown by a finger stops where it was let go rather than coasting on. */
  still: boolean;
  /**
   * The finger has panned `travel` past the picture's side, along the inline axis and positive
   * toward its end, as a swipe's travel is. Zero once it has come back.
   */
  onCarry: (travel: number) => void;
  /** The finger lifted `travel` past the side, moving at `velocity` in pixels per millisecond. */
  onCarryEnd: (travel: number, velocity: number) => void;
  /** The carry was broken off: a second finger touched, or the browser took the touch. */
  onCarryCancel: () => void;
}

/** What fingers are doing to the picture, from when they take it until the last one lifts. */
interface Gesture {
  readonly frame: Frame;
  /** The stage's centre on the screen, which every point is measured from. */
  readonly origin: Point;
  /** 1 where the inline axis's end is the right, -1 where it is the left. */
  readonly flip: number;
  /** The view asked for when the fingers took their present grip, and that grip. */
  from: View;
  grip: Grip;
  /** The view the fingers ask for now, which may lie past the picture's limits. */
  asked: View;
  shown: View;
  /** The fingers' middle now. */
  about: Point;
  /** The travel last handed to `onCarry`, or `null` when none has been. */
  carried: number | null;
  /** One finger's recent positions, for the throw when it lifts. */
  readonly x: VelocityTracker;
  readonly y: VelocityTracker;
}

/**
 * Zooms the gallery's current picture under fingers on `target`'s element, for as long as
 * `enabled`. Two fingers pinch it about their middle, and pan it as they move together; on a
 * zoomed picture one finger pans, and thrown, the picture coasts; a double-tap on the picture
 * zooms in on the point tapped, or back out. `picture` is the picture and `stage` the box it
 * is fitted to, which it may be panned across but never out of: zoomed past a limit or panned
 * past an edge it gives a little, and eases back when the fingers lift (`galleryZoom.ts`).
 * Fingers that land on a button or a link are theirs.
 *
 * One finger on a fitted picture is left alone, for the gallery's swipe. Touch events, as
 * `useSwipe` reads, so that the two agree on whose a finger is.
 */
export function useGalleryZoom(
  target: RefObject<HTMLElement | null>,
  stage: RefObject<HTMLElement | null>,
  picture: RefObject<HTMLImageElement | null>,
  handlers: ZoomHandlers,
  enabled: boolean,
): void {
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });

  useEffect(() => {
    const root = target.current;
    if (root === null || !enabled) {
      return;
    }
    /** Where each finger on the gallery is, by its touch's identifier, first down first. */
    const fingers = new Map<number, Point>();
    let gesture: Gesture | null = null;
    /** One finger that has touched and not yet moved: a tap, if it lifts soon and in place. */
    let press: { id: number; x: number; y: number; at: number; onPicture: boolean } | null = null;
    let lastTap: { x: number; y: number; at: number } | null = null;
    let coasting = 0;

    const stopCoasting = () => {
      cancelAnimationFrame(coasting);
      coasting = 0;
    };
    const coast = (from: View, thrown: Point, frame: Frame) => {
      let view = from;
      let velocity = thrown;
      let last = performance.now();
      const step = (now: number) => {
        // A frame's time may be a moment before the touch that started the coast was handled.
        const next = coasted(view, velocity, Math.max(0, now - last), frame);
        last = now;
        view = next.view;
        velocity = next.velocity;
        if (Math.hypot(velocity.x, velocity.y) < COASTING_STOPS) {
          coasting = 0;
          latest.current.onView(view, "resting");
          return;
        }
        latest.current.onView(view, "held");
        coasting = requestAnimationFrame(step);
      };
      coasting = requestAnimationFrame(step);
    };

    /** Brings `fingers` in step with the touches on the screen: where each is, and which are left. */
    const sync = (touches: TouchList) => {
      const down = new Map(Array.from(touches, (touch) => [touch.identifier, touch]));
      for (const id of fingers.keys()) {
        const touch = down.get(id);
        if (touch === undefined) {
          fingers.delete(id);
        } else {
          fingers.set(id, { x: touch.clientX, y: touch.clientY });
        }
      }
    };
    const gripOf = (origin: Point): Grip => {
      const [first, second] = fingers.values();
      if (first === undefined) {
        return { centre: { x: 0, y: 0 }, span: 0 };
      }
      if (second === undefined) {
        return { centre: { x: first.x - origin.x, y: first.y - origin.y }, span: 0 };
      }
      return {
        centre: {
          x: (first.x + second.x) / 2 - origin.x,
          y: (first.y + second.y) / 2 - origin.y,
        },
        span: Math.hypot(second.x - first.x, second.y - first.y),
      };
    };
    /** The fingers take the picture as it is drawn now, or `null` when there is none to take. */
    const begin = (): Gesture | null => {
      const measured = measure(stage.current, picture.current);
      if (measured === null) {
        return null;
      }
      const grip = gripOf(measured.origin);
      return {
        frame: measured.frame,
        origin: measured.origin,
        flip: getComputedStyle(root).direction === "rtl" ? -1 : 1,
        from: measured.drawn,
        grip,
        asked: measured.drawn,
        shown: measured.drawn,
        about: grip.centre,
        carried: null,
        x: new VelocityTracker(),
        y: new VelocityTracker(),
      };
    };
    /** The fingers holding the picture have changed: it goes on from where they left it. */
    const regrip = (taken: Gesture) => {
      if (taken.carried !== null) {
        // What had gone past the side went to the gallery, which takes it back.
        latest.current.onCarryCancel();
        taken.carried = null;
        taken.from = { ...taken.asked, x: taken.shown.x };
      } else {
        taken.from = taken.asked;
      }
      taken.asked = taken.from;
      taken.grip = gripOf(taken.origin);
      taken.about = taken.grip.centre;
      taken.x.reset();
      taken.y.reset();
    };
    const follow = (moving: Gesture, at: number) => {
      const grip = gripOf(moving.origin);
      moving.asked = moved(moving.from, moving.grip, grip);
      moving.about = grip.centre;
      const carry = fingers.size === 1 && latest.current.carries;
      const { view, beyond } = held(moving.asked, moving.about, moving.frame, carry);
      moving.shown = view;
      latest.current.onView(view, "held");
      if (carry && (beyond !== 0 || moving.carried !== null)) {
        moving.carried = beyond * moving.flip;
        latest.current.onCarry(moving.carried);
      }
      if (fingers.size === 1) {
        moving.x.add(grip.centre.x, at);
        moving.y.add(grip.centre.y, at);
      }
    };
    /** The last finger has lifted, or with `broken` the browser has taken the touch. */
    const finish = (ended: Gesture, at: number, broken: boolean) => {
      const limit = (speed: number) => Math.max(-MAX_VELOCITY, Math.min(MAX_VELOCITY, speed));
      const thrown = broken
        ? { x: 0, y: 0 }
        : { x: limit(ended.x.velocity(at)), y: limit(ended.y.velocity(at)) };
      const rest = rested(ended.asked, ended.about, ended.frame);
      if (ended.carried !== null) {
        if (broken) {
          latest.current.onCarryCancel();
        } else {
          latest.current.onCarryEnd(ended.carried, thrown.x * ended.flip);
        }
      }
      if (!sameView(rest, ended.shown)) {
        latest.current.onView(rest, "easing");
      } else if (
        ended.carried === null &&
        !latest.current.still &&
        Math.hypot(thrown.x, thrown.y) >= COASTING_STOPS
      ) {
        coast(rest, thrown, ended.frame);
      } else {
        latest.current.onView(rest, "resting");
      }
    };

    const onTouchStart = (event: TouchEvent) => {
      stopCoasting();
      sync(event.touches);
      if (event.target instanceof Element && event.target.closest("button, a") !== null) {
        return;
      }
      for (const touch of event.changedTouches) {
        fingers.set(touch.identifier, { x: touch.clientX, y: touch.clientY });
      }
      const touch = event.changedTouches[0];
      if (fingers.size === 1 && touch !== undefined) {
        press = {
          id: touch.identifier,
          x: touch.clientX,
          y: touch.clientY,
          at: event.timeStamp,
          onPicture: event.target === picture.current,
        };
        return;
      }
      press = null;
      lastTap = null;
      if (gesture === null) {
        gesture = begin();
      } else {
        regrip(gesture);
      }
    };
    const onTouchMove = (event: TouchEvent) => {
      sync(event.touches);
      if (gesture === null) {
        const moving = press === null ? undefined : fingers.get(press.id);
        if (
          press === null ||
          moving === undefined ||
          Math.hypot(moving.x - press.x, moving.y - press.y) < SLOP_PX
        ) {
          return;
        }
        press = null;
        lastTap = null;
        // On a fitted picture the finger is the gallery's, to swipe with. The picture is taken
        // from where the finger is now, so it does not jump the slop's length as it starts.
        const zoomed = (measure(stage.current, picture.current)?.drawn.scale ?? 1) > 1;
        gesture = zoomed && event.cancelable ? begin() : null;
        if (gesture === null) {
          return;
        }
      }
      if (event.cancelable) {
        event.preventDefault();
      }
      follow(gesture, event.timeStamp);
    };
    const onTouchEnd = (event: TouchEvent) => {
      const lifted = Array.from(event.changedTouches).filter((touch) =>
        fingers.delete(touch.identifier),
      );
      const broken = event.type === "touchcancel";
      if (gesture !== null) {
        if (fingers.size > 0) {
          regrip(gesture);
          return;
        }
        const ended = gesture;
        gesture = null;
        if (!broken && event.cancelable) {
          // No click for fingers that moved the picture, whatever they lifted over.
          event.preventDefault();
        }
        finish(ended, event.timeStamp, broken);
        return;
      }
      const pressed = press;
      press = null;
      const touch =
        pressed === null ? undefined : lifted.find((each) => each.identifier === pressed.id);
      if (
        broken ||
        pressed === null ||
        touch === undefined ||
        !pressed.onPicture ||
        event.timeStamp - pressed.at > TAP_MS ||
        Math.hypot(touch.clientX - pressed.x, touch.clientY - pressed.y) >= SLOP_PX
      ) {
        lastTap = null;
        return;
      }
      const tap = { x: touch.clientX, y: touch.clientY, at: event.timeStamp };
      const second =
        lastTap !== null &&
        tap.at - lastTap.at <= TAP_MS &&
        Math.hypot(tap.x - lastTap.x, tap.y - lastTap.y) <= DOUBLE_TAP_PX;
      const measured = second ? measure(stage.current, picture.current) : null;
      if (measured === null) {
        lastTap = tap;
        return;
      }
      lastTap = null;
      const about = { x: tap.x - measured.origin.x, y: tap.y - measured.origin.y };
      latest.current.onView(doubleTapped(measured.drawn, about, measured.frame), "easing");
    };

    root.addEventListener("touchstart", onTouchStart, { passive: true });
    // Captured, as the gallery's swipe captures them, so every move is heard here too.
    root.addEventListener("touchmove", onTouchMove, { passive: false, capture: true });
    root.addEventListener("touchend", onTouchEnd, { passive: false });
    root.addEventListener("touchcancel", onTouchEnd);
    return () => {
      root.removeEventListener("touchstart", onTouchStart);
      root.removeEventListener("touchmove", onTouchMove, { capture: true });
      root.removeEventListener("touchend", onTouchEnd);
      root.removeEventListener("touchcancel", onTouchEnd);
      stopCoasting();
      if (gesture !== null && gesture.carried !== null) {
        latest.current.onCarryCancel();
      }
    };
  }, [target, stage, picture, enabled]);
}

/**
 * What a gesture starts from: the sizes the picture is zoomed within, the stage's centre on
 * the screen, and the view the picture is drawn at this moment, part-way through an ease
 * included. `null` while there is no picture to zoom, as before it has loaded.
 */
function measure(
  stage: HTMLElement | null,
  picture: HTMLImageElement | null,
): { frame: Frame; origin: Point; drawn: View } | null {
  if (stage === null || picture === null || picture.naturalWidth === 0) {
    return null;
  }
  const fitted = { width: picture.offsetWidth, height: picture.offsetHeight };
  if (fitted.width === 0 || fitted.height === 0) {
    return null;
  }
  const box = stage.getBoundingClientRect();
  // The picture is drawn at `translate(x, y) scale(scale)` about its centre.
  const matrix = new DOMMatrixReadOnly(getComputedStyle(picture).transform);
  return {
    frame: {
      stage: { width: box.width, height: box.height },
      fitted,
      natural: { width: picture.naturalWidth, height: picture.naturalHeight },
    },
    origin: { x: box.left + box.width / 2, y: box.top + box.height / 2 },
    drawn: { scale: matrix.a, x: matrix.e, y: matrix.f },
  };
}
