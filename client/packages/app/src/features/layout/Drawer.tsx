import { Capacitor, type PluginListenerHandle } from "@capacitor/core";
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
  type RefObject,
} from "react";
import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMotion } from "@/features/layout/motion";
import { useSwipe } from "@/features/layout/useSwipe";

/**
 * Where a drawer is: away; following a finger, `shown` being how much of it is out (0 to 1);
 * drawn away, about to slide in; in; or sliding away from `shown`.
 */
type Phase =
  | { readonly kind: "closed" }
  | { readonly kind: "dragging"; readonly shown: number }
  | { readonly kind: "opening" }
  | { readonly kind: "open" }
  | { readonly kind: "closing"; readonly shown: number };

/** How fast a finger must be moving as it lifts to throw the drawer its way, in px/ms. */
const FLICK = 0.3;

/** Whether a drawer in `phase` counts as open: out, or on its way. */
function isOut(phase: Phase): boolean {
  return phase.kind === "opening" || phase.kind === "open";
}

/** Which edge of the screen a drawer comes from. */
export type DrawerEdge = "end" | "bottom";

/**
 * A drawer at an edge of the screen: a modal over a backdrop that darkens as it comes out. At
 * the inline `end` it is a panel titled `title`, which a finger draws out by swiping toward
 * the inline start across `swipeFrom`'s element, while `enabled`, and puts away by swiping
 * back toward the end. At the `bottom` it is a sheet as tall as what it holds, named `title`
 * for assistive technology alone, with a handle to show it can be pulled down, which puts it
 * away. It follows the finger, and when the finger lifts it goes the way the finger was
 * thrown, or, if the finger had come to rest, whichever way it is more than half. Opened
 * through `isOpen`, it slides all the way; a tap on the backdrop, Escape, the panel's X, or
 * the Android back button puts it away. Where motion is reduced it fades in and out in place,
 * and only a finger moves it.
 */
export function Drawer({
  edge = "end",
  swipeFrom,
  enabled = true,
  isOpen,
  onOpenChange,
  title,
  children,
}: {
  edge?: DrawerEdge;
  /** Where a finger swipes to draw an `end` drawer out; none draws it out by finger. */
  swipeFrom?: RefObject<HTMLElement | null>;
  enabled?: boolean;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: ReactNode;
}) {
  const motion = useMotion();
  const [phase, setPhase] = useState<Phase>({ kind: "closed" });
  // Closed as first seen, so a drawer drawn already open slides in like one opened later.
  const [seenOpen, setSeenOpen] = useState(false);
  const overlay = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const nowhere = useRef<HTMLElement>(null);
  const bottom = edge === "bottom";

  /** Moves to `next`, telling the owner when that opens or closes the drawer. */
  const go = (next: Phase) => {
    setPhase(next);
    if (isOut(next) !== isOpen) {
      onOpenChange(isOut(next));
    }
  };
  const closeFrom = (from: Phase) => {
    if (from.kind === "open" || from.kind === "opening") {
      go({ kind: "closing", shown: 1 });
    } else if (from.kind === "dragging") {
      go({ kind: "closing", shown: from.shown });
    }
  };

  if (!enabled && phase.kind !== "closed") {
    setPhase({ kind: "closed" });
  }
  // The owner opening or closing it.
  if (isOpen !== seenOpen) {
    setSeenOpen(isOpen);
    if (isOpen && !isOut(phase)) {
      setPhase({ kind: "opening" });
    } else if (!isOpen && isOut(phase)) {
      setPhase({ kind: "closing", shown: 1 });
    }
  }

  /** What share of the drawer `travel` pixels is. */
  const share = (travel: number) => {
    const size = bottom
      ? (panel.current?.offsetHeight ?? window.innerHeight / 2)
      : (panel.current?.offsetWidth ?? drawerWidth(window.innerWidth));
    return Math.min(1, Math.max(0, travel / size));
  };
  const settle = (shown: number, velocity: number) => {
    const opens = velocity > FLICK || (velocity >= -FLICK && shown > 0.5);
    go(opens ? { kind: "open" } : { kind: "closing", shown });
  };
  useSwipe(
    swipeFrom ?? nowhere,
    {
      onStart: (toward) => toward === "start" && phase.kind === "closed",
      onMove: (travel) => {
        setPhase({ kind: "dragging", shown: share(-travel) });
      },
      onEnd: (travel, velocity) => {
        settle(share(-travel), -velocity);
      },
      onCancel: () => {
        settle(0, 0);
      },
    },
    enabled && swipeFrom !== undefined,
  );
  useSwipe(
    overlay,
    {
      onStart: (toward) => toward === "end" && phase.kind === "open",
      onMove: (travel) => {
        setPhase({ kind: "dragging", shown: 1 - share(travel) });
      },
      onEnd: (travel, velocity) => {
        settle(1 - share(travel), -velocity);
      },
      onCancel: () => {
        go({ kind: "open" });
      },
    },
    enabled && phase.kind !== "closed",
    bottom ? "block" : "inline",
  );

  // Drawn first where it is away, so that it slides in from there on the next frame.
  useEffect(() => {
    if (phase.kind !== "opening") {
      return;
    }
    void panel.current?.getBoundingClientRect();
    const frame = requestAnimationFrame(() => {
      setPhase({ kind: "open" });
    });
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [phase.kind]);
  // Taken away once it has slid there; at once where nothing moves.
  useEffect(() => {
    if (phase.kind !== "closing") {
      return;
    }
    let cancelled = false;
    const sliding = (overlay.current?.getAnimations({ subtree: true }) ?? []).filter(
      (animation) => animation instanceof CSSTransition,
    );
    void Promise.allSettled(sliding.map((animation) => animation.finished)).then(() => {
      if (!cancelled) {
        setPhase({ kind: "closed" });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [phase.kind]);

  // While any listener is registered Capacitor leaves the back button to it alone, so one is
  // registered only while the drawer is out.
  const out = phase.kind !== "closed" && phase.kind !== "closing";
  const latestClose = useRef<() => void>(() => undefined);
  useEffect(() => {
    latestClose.current = () => {
      closeFrom(phase);
    };
  });
  useEffect(() => {
    if (!out || !Capacitor.isNativePlatform()) {
      return;
    }
    let cancelled = false;
    let back: PluginListenerHandle | null = null;
    void import("@capacitor/app")
      .then(({ App }) =>
        App.addListener("backButton", () => {
          latestClose.current();
        }),
      )
      .then((handle) => {
        if (cancelled) {
          void handle.remove();
        } else {
          back = handle;
        }
      });
    return () => {
      cancelled = true;
      void back?.remove();
    };
  }, [out]);

  // How far out the panel is drawn, whether it is faded, how dark the backdrop is, and what
  // eases there.
  const drawn =
    phase.kind === "dragging" || (phase.kind === "closing" && motion.reduced)
      ? phase.shown
      : phase.kind === "open" || (phase.kind === "opening" && motion.reduced)
        ? 1
        : 0;
  const faded = motion.reduced && (phase.kind === "opening" || phase.kind === "closing");
  const darkness = phase.kind === "dragging" ? phase.shown : phase.kind === "open" ? 1 : 0;
  const transition =
    phase.kind === "dragging"
      ? "none"
      : motion.reduced
        ? "opacity var(--motion-base) var(--motion-ease-out)"
        : "transform var(--motion-slow) var(--motion-ease-out), opacity var(--motion-slow) var(--motion-ease-out)";
  const panelStyle: CSSProperties = {
    // Away is off the end: rightward left to right, leftward right to left (`--drawer-away`);
    // or below the bottom.
    transform: bottom
      ? `translateY(calc(${String(1 - drawn)} * 100%))`
      : `translateX(calc(${String(1 - drawn)} * var(--drawer-away)))`,
    opacity: faded ? 0 : 1,
    transition,
  };
  return (
    <ModalOverlay
      ref={overlay}
      isOpen={phase.kind !== "closed"}
      onOpenChange={(open) => {
        if (!open) {
          closeFrom(phase);
        }
      }}
      isDismissable
      className="fixed inset-0 z-10"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-black/40"
        style={{ opacity: darkness, transition }}
      />
      <Modal ref={panel} className={bottom ? bottomSheetClass : endPanelClass} style={panelStyle}>
        {bottom ? (
          <Dialog aria-label={title} className="flex min-h-0 flex-1 flex-col outline-none">
            <div aria-hidden="true" className="flex shrink-0 justify-center pt-2 pb-1">
              <div className="h-1 w-10 rounded-full bg-line forced-fill" />
            </div>
            {children}
          </Dialog>
        ) : (
          <Dialog className="flex min-h-0 flex-1 flex-col outline-none">
            <div className="px-4 pt-4 pb-2">
              <DialogHeading>{title}</DialogHeading>
            </div>
            {children}
          </Dialog>
        )}
      </Modal>
    </ModalOverlay>
  );
}

/** A drawer at the inline end: 20rem wide, leaving at least 3rem of the screen beside it. */
const endPanelClass =
  "absolute inset-y-0 end-0 flex w-[min(20rem,100%_-_3rem)] flex-col border-s border-line " +
  "bg-surface-raised pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] shadow-xl " +
  "outline-none [--drawer-away:100%] ltr:pe-[env(safe-area-inset-right)] " +
  "rtl:pe-[env(safe-area-inset-left)] rtl:[--drawer-away:-100%]";
/**
 * A drawer at the bottom: the screen's width, as tall as what it holds up to most of the
 * screen, and clear of the home indicator and the sides of a phone held sideways.
 */
const bottomSheetClass =
  "absolute inset-x-0 bottom-0 flex max-h-[85%] flex-col rounded-t-xl border-t border-line " +
  "bg-surface-raised pb-[env(safe-area-inset-bottom)] shadow-xl outline-none " +
  "ltr:ps-[env(safe-area-inset-left)] ltr:pe-[env(safe-area-inset-right)] " +
  "rtl:ps-[env(safe-area-inset-right)] rtl:pe-[env(safe-area-inset-left)]";

/**
 * How wide a drawer is drawn on a screen this wide, in CSS pixels, as its class has it: 20rem,
 * leaving at least 3rem of the screen beside it. A finger's first move comes before the
 * drawer is drawn to be measured.
 */
function drawerWidth(screen: number): number {
  return Math.min(320, screen - 48);
}
