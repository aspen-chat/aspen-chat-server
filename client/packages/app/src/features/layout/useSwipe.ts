import { useEffect, useRef, type RefObject } from "react";
import { VelocityTracker } from "@/features/messages/scrollPhysics";

/** How far a finger moves before a swipe is told from a scroll or a tap, in CSS pixels. */
const SLOP_PX = 10;
/** How much further along the swipe's axis than across it a finger must have moved. */
const ALONG_RATIO = 1.2;

/**
 * Which way a swipe goes: across the screen (`inline`, the way text runs) or up and down it
 * (`block`).
 */
export type SwipeAxis = "inline" | "block";

/**
 * A side of the swipe's axis: on the inline axis the start is the left in a left-to-right
 * layout; on the block axis it is the top.
 */
export type SwipeSide = "start" | "end";

export interface SwipeHandlers {
  /**
   * A finger has begun to move along the axis, `toward` one side. Returns whether the swipe is
   * taken; a finger whose swipe is not is left to whatever else wants it.
   */
  onStart: (toward: SwipeSide) => boolean;
  /** The finger's travel since it touched, along the axis, positive toward the end. */
  onMove: (travel: number) => void;
  /** The finger lifted, moving at `velocity` along the same axis, in pixels per millisecond. */
  onEnd: (travel: number, velocity: number) => void;
  /** The swipe was broken off: a second finger touched, or the browser took the touch. */
  onCancel: () => void;
}

/**
 * Follows a finger swiping across `target`'s element along `axis`, for as long as `enabled`.
 * Until it has moved `SLOP_PX` the finger is nobody's; then, if it moved mostly along the
 * axis, and nothing it touched scrolls that way (a wide code block, say, or a list scrolled
 * down from its top), and `onStart` takes it, it is the swipe's to its end: the browser does
 * not scroll for it, the touch moves no longer reach what lies under the finger (the message
 * list follows a finger itself on iOS), and lifting it is not a tap. A finger that moved
 * mostly across the axis is left to scroll.
 *
 * Touch events, not pointer events: the browser cancels a pointer when it begins to scroll for
 * it, before the swipe could be told from a scroll, and a touch move can be refused only while
 * the browser has not yet begun to (`cancelable`).
 */
export function useSwipe(
  target: RefObject<HTMLElement | null>,
  handlers: SwipeHandlers,
  enabled: boolean,
  axis: SwipeAxis = "inline",
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
    let finger: {
      id: number;
      x: number;
      y: number;
      from: EventTarget | null;
      /** 1 where the axis's end is the right or the bottom, -1 where it is the left. */
      flip: number;
      swiping: boolean;
      travel: number;
      tracker: VelocityTracker;
    } | null = null;

    const breakOff = () => {
      if (finger?.swiping === true) {
        latest.current.onCancel();
      }
      finger = null;
    };
    const onTouchStart = (event: TouchEvent) => {
      const touch = event.touches[0];
      if (event.touches.length !== 1 || touch === undefined) {
        breakOff();
        return;
      }
      finger = {
        id: touch.identifier,
        x: touch.clientX,
        y: touch.clientY,
        from: event.target,
        flip: axis === "inline" && getComputedStyle(root).direction === "rtl" ? -1 : 1,
        swiping: false,
        travel: 0,
        tracker: new VelocityTracker(),
      };
    };
    const onTouchMove = (event: TouchEvent) => {
      const touch = finger === null ? undefined : ownTouch(event.touches, finger.id);
      if (finger === null || touch === undefined) {
        return;
      }
      const dx = touch.clientX - finger.x;
      const dy = touch.clientY - finger.y;
      const [along, across] = axis === "inline" ? [dx, dy] : [dy, dx];
      if (!finger.swiping) {
        if (Math.hypot(dx, dy) < SLOP_PX) {
          return;
        }
        const taken =
          event.cancelable &&
          Math.abs(along) > Math.abs(across) * ALONG_RATIO &&
          !scrollsAlong(finger.from, root, axis, along) &&
          latest.current.onStart(along * finger.flip > 0 ? "end" : "start");
        if (!taken) {
          finger = null;
          return;
        }
        finger.swiping = true;
      }
      event.preventDefault();
      event.stopPropagation();
      finger.travel = along * finger.flip;
      finger.tracker.add(finger.travel, event.timeStamp);
      latest.current.onMove(finger.travel);
    };
    const onTouchEnd = (event: TouchEvent) => {
      if (finger === null || ownTouch(event.changedTouches, finger.id) === undefined) {
        return;
      }
      if (finger.swiping) {
        // No click for a finger that swiped, whatever it lifted over.
        event.preventDefault();
        latest.current.onEnd(finger.travel, finger.tracker.velocity(event.timeStamp));
      }
      finger = null;
    };

    root.addEventListener("touchstart", onTouchStart, { passive: true });
    // Captured, so a move the swipe takes stops here, before what is under the finger hears it.
    root.addEventListener("touchmove", onTouchMove, { passive: false, capture: true });
    root.addEventListener("touchend", onTouchEnd, { passive: false });
    root.addEventListener("touchcancel", breakOff);
    return () => {
      root.removeEventListener("touchstart", onTouchStart);
      root.removeEventListener("touchmove", onTouchMove, { capture: true });
      root.removeEventListener("touchend", onTouchEnd);
      root.removeEventListener("touchcancel", breakOff);
      breakOff();
    };
  }, [target, enabled, axis]);
}

function ownTouch(touches: TouchList, id: number): Touch | undefined {
  return Array.from(touches).find((touch) => touch.identifier === id);
}

/**
 * Whether anything from `from` up to `root` scrolls along `axis` and has room to go the way a
 * finger moving `delta` along it would take it: leftward fingers reveal what is to the right,
 * and downward ones what is above.
 */
function scrollsAlong(
  from: EventTarget | null,
  root: HTMLElement,
  axis: SwipeAxis,
  delta: number,
): boolean {
  for (
    let element = from instanceof Element ? from : null;
    element !== null && element !== root;
    element = element.parentElement
  ) {
    const room =
      axis === "inline"
        ? element.scrollWidth - element.clientWidth
        : element.scrollHeight - element.clientHeight;
    if (room <= 0) {
      continue;
    }
    const style = getComputedStyle(element);
    const overflow = axis === "inline" ? style.overflowX : style.overflowY;
    if (overflow !== "auto" && overflow !== "scroll") {
      continue;
    }
    // `scrollLeft` runs from 0 at the inline start: up to `room` left to right, down to
    // `-room` right to left. `scrollTop` runs from 0 at the top to `room`.
    const scrolled =
      axis === "block"
        ? element.scrollTop
        : style.direction === "rtl"
          ? room + element.scrollLeft
          : element.scrollLeft;
    if (delta < 0 ? scrolled < room - 1 : scrolled > 1) {
      return true;
    }
  }
  return false;
}
