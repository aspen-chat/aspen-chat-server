import { MOTION_SPEED } from "@aspen/protocol";
import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { usePreference } from "@/api/hooks";
import { useMediaQuery } from "@/features/layout/useMediaQuery";

const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

/** How the app may move, for motion made in code rather than CSS (`styles.css`, Motion). */
export interface Motion {
  /** Animations are off: everything happens at once. */
  readonly off: boolean;
  /** This device asks to reduce motion: things may fade, but not travel or grow. */
  readonly reduced: boolean;
  /** How much longer than normal each animation takes (0.5 at twice the speed). */
  readonly scale: number;
}

export function useMotion(): Motion {
  const speed = usePreference(MOTION_SPEED);
  const reduced = useMediaQuery(REDUCED_MOTION);
  return { off: speed === 0, reduced, scale: speed === 0 ? 0 : 1 / speed };
}

/**
 * Keeps <html> in step with the reader's animation speed: `--motion-scale`, which every motion
 * token in `styles.css` multiplies, and `data-motion="off"` when animations are off.
 */
export function useFollowMotionSpeed(): void {
  const { off, scale } = useMotion();
  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--motion-scale", String(scale));
    if (off) {
      root.dataset.motion = "off";
    } else {
      delete root.dataset.motion;
    }
  }, [off, scale]);
}

/**
 * A count of the times `value` has grown since this was first rendered, to key an element with
 * `motion-pop` so it swells each time (a new key starts its animation again). It starts at 0,
 * when nothing has grown and nothing should pop.
 */
export function useGrowthKey(value: number): number {
  const [seen, setSeen] = useState(value);
  const [grown, setGrown] = useState(0);
  if (value !== seen) {
    setSeen(value);
    if (value > seen) {
      setGrown(grown + 1);
    }
  }
  return grown;
}

/**
 * Glides a list's rows to their new places when their order changes, as when a drag drops one
 * or a folder opens above them, rather than letting them jump: each row that moved is drawn
 * from where it was, and eases into place. Rows are the list's children with a `data-key`, as
 * React Aria's collections mark them; `order` names their order, and changes with it.
 */
export function useReorderGlide(list: RefObject<HTMLElement | null>, order: string): void {
  const motion = useMotion();
  const tops = useRef(new Map<string, number>());
  useLayoutEffect(() => {
    const element = list.current;
    if (element === null) {
      return;
    }
    const next = new Map<string, number>();
    for (const row of element.querySelectorAll<HTMLElement>(":scope > [data-key]")) {
      const key = row.dataset.key;
      if (key === undefined) {
        continue;
      }
      const top = row.offsetTop;
      next.set(key, top);
      const was = tops.current.get(key);
      if (!motion.off && !motion.reduced && was !== undefined && was !== top) {
        row.animate([{ transform: `translateY(${String(was - top)}px)` }, { transform: "none" }], {
          duration: GLIDE_MS * motion.scale,
          easing: "cubic-bezier(0.2, 0, 0, 1)",
        });
      }
    }
    tops.current = next;
    // Measured when the order changes, and only then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [order]);
}

/** How long a row glides at normal speed, as `--motion-base` in `styles.css`. */
const GLIDE_MS = 200;
