import { useLayoutEffect, useRef } from "react";
import { useMotion } from "@/features/layout/motion";
import type { Departing } from "@/features/messages/departing";

/** How long a deleted message's space takes to close at normal speed, as `--motion-base`. */
const COLLAPSE_MS = 200;

/**
 * The space a deleted message leaves, closing over a moment; the list's `gap-1` between messages
 * closes with it. The closing starts a frame after the list has rendered without the message,
 * not as the space is made: a long render would otherwise spend most of it before anything is
 * painted.
 */
export function DepartingSpace({ gone, onClosed }: { gone: Departing; onClosed: () => void }) {
  const space = useRef<HTMLDivElement>(null);
  const motion = useMotion();
  useLayoutEffect(() => {
    const element = space.current;
    if (element === null) {
      return;
    }
    let closing: Animation | undefined;
    // Two frames on: the frame the space is made in may have begun long before a slow render
    // ended, and an animation started in it would count from then.
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(start);
    });
    const start = () => {
      closing = element.animate(
        [
          { height: `${String(gone.height)}px`, marginTop: "0px" },
          { height: "0px", marginTop: "-0.25rem" },
        ],
        {
          duration: COLLAPSE_MS * motion.scale,
          easing: "cubic-bezier(0.4, 0, 0.2, 1)",
          fill: "forwards",
        },
      );
      closing.finished.then(onClosed, () => undefined);
    };
    return () => {
      cancelAnimationFrame(frame);
      closing?.cancel();
    };
    // Closes once, from where it was made.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div
      ref={space}
      aria-hidden="true"
      className="overflow-hidden"
      style={{ height: `${String(gone.height)}px` }}
    />
  );
}
