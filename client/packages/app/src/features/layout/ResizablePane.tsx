import {
  createContext,
  useContext,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";
import { usePreference, useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { MEDIUM_SCREEN, useMediaQuery } from "@/features/layout/useMediaQuery";
import type { PaneSizing } from "@/features/layout/paneSizes";

/** The edge of the pane a landmark is in, which it renders with `PaneEdge`. */
const EdgeContext = createContext<ReactNode>(null);

/**
 * The movable edge of the pane this is in, rendered by the landmark that fills the pane (a
 * sidebar's `section`, the member list's `aside`, the thread's `section`), inside it, so that
 * the edge, like everything else on the page, belongs to a landmark. The landmark is
 * `relative`, which places the edge along its side.
 */
export function PaneEdge() {
  return useContext(EdgeContext);
}

/** How far an arrow key moves the edge, and with Shift. */
const STEP = 16;
const BIG_STEP = 64;

function clamp(sizing: PaneSizing, width: number): number {
  return Math.round(Math.min(sizing.max, Math.max(sizing.min, width)));
}

/**
 * A pane beside the channel, as wide as the reader last made it. Its edge toward the channel
 * (`edge`: the inline end of a pane at the start, the inline start of one at the end) is a
 * separator: dragged, or moved with the arrow keys (Shift for bigger steps), Home and End for
 * the narrowest and widest, and a double-click or Enter for the default. The width is kept on
 * this device when a drag ends, not as it moves. Where there is room for one pane only, the
 * pane fills the screen and has no edge to move. The pane's landmark renders the edge
 * (`PaneEdge`).
 */
export function ResizablePane({
  sizing,
  edge,
  label,
  className = "",
  role,
  children,
}: {
  sizing: PaneSizing;
  edge: "start" | "end";
  /** What the pane is, for the separator's name. */
  label: string;
  className?: string;
  role?: string | undefined;
  children: ReactNode;
}) {
  const m = useMessages();
  const sync = useSync();
  const wide = useMediaQuery(MEDIUM_SCREEN);
  const kept = usePreference(sizing.definition);
  const [dragging, setDragging] = useState<number | null>(null);
  const drag = useRef<{ x: number; width: number; flip: number } | null>(null);
  const width = clamp(sizing, dragging ?? kept?.width ?? sizing.fallback);

  const keep = (next: number) => {
    void sync.preferences.set(sizing.definition, {
      version: sizing.version,
      width: clamp(sizing, next),
    });
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    const rtl = getComputedStyle(event.currentTarget).direction === "rtl";
    // Moving the edge outward widens the pane: rightward for a start pane in a left-to-right
    // layout, leftward for an end pane, and the other way round right to left.
    const flip = (edge === "end" ? 1 : -1) * (rtl ? -1 : 1);
    drag.current = { x: event.clientX, width, flip };
    setDragging(width);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    if (start !== null) {
      setDragging(clamp(sizing, start.width + (event.clientX - start.x) * start.flip));
    }
  };
  const onPointerUp = () => {
    if (drag.current !== null && dragging !== null) {
      keep(dragging);
    }
    drag.current = null;
    setDragging(null);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? BIG_STEP : STEP;
    const rtl = getComputedStyle(event.currentTarget).direction === "rtl";
    const outward = (edge === "end") !== rtl ? "ArrowRight" : "ArrowLeft";
    const inward = outward === "ArrowRight" ? "ArrowLeft" : "ArrowRight";
    const next =
      event.key === outward
        ? width + step
        : event.key === inward
          ? width - step
          : event.key === "Home"
            ? sizing.min
            : event.key === "End"
              ? sizing.max
              : event.key === "Enter"
                ? sizing.fallback
                : null;
    if (next !== null) {
      event.preventDefault();
      keep(next);
    }
  };

  const handle = wide ? (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={format(m.layout.resizePane, { pane: label })}
      aria-valuemin={sizing.min}
      aria-valuemax={sizing.max}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={() => {
        keep(sizing.fallback);
      }}
      onKeyDown={onKeyDown}
      className={
        "group/edge absolute inset-y-0 z-10 w-2 cursor-col-resize touch-none outline-none " +
        (edge === "end" ? "-end-1" : "-start-1")
      }
    >
      <div
        className={
          "mx-auto h-full w-0.5 transition-colors group-hover/edge:bg-accent/60 group-focus-visible/edge:bg-accent " +
          (dragging !== null ? "bg-accent" : "")
        }
      />
    </div>
  ) : null;

  return (
    <div
      role={role}
      className={"relative md:shrink-0 " + className}
      style={wide ? { width } : undefined}
    >
      <EdgeContext.Provider value={handle}>{children}</EdgeContext.Provider>
    </div>
  );
}
