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

/**
 * A drawer at the inline end of the screen, titled `title`: a modal over a backdrop that
 * darkens as it comes out. A finger draws it out by swiping toward the inline start across
 * `swipeFrom`'s element, while `enabled`, and puts it away by swiping back toward the end. It
 * follows the finger, and when the finger lifts it goes the way the finger was thrown, or, if
 * the finger had come to rest, whichever way it is more than half. Opened through `isOpen`, it
 * slides all the way; a tap on the backdrop, Escape, its X, or the Android back button puts it
 * away. Where motion is reduced it fades in and out in place, and only a finger moves it.
 */
export function Drawer({
  swipeFrom,
  enabled,
  isOpen,
  onOpenChange,
  title,
  children,
}: {
  swipeFrom: RefObject<HTMLElement | null>;
  enabled: boolean;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: ReactNode;
}) {
  const motion = useMotion();
  const [phase, setPhase] = useState<Phase>({ kind: "closed" });
  const [seenOpen, setSeenOpen] = useState(isOpen);
  const overlay = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);

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
  const share = (travel: number) =>
    Math.min(
      1,
      Math.max(0, travel / (panel.current?.offsetWidth ?? drawerWidth(window.innerWidth))),
    );
  const settle = (shown: number, velocity: number) => {
    const opens = velocity > FLICK || (velocity >= -FLICK && shown > 0.5);
    go(opens ? { kind: "open" } : { kind: "closing", shown });
  };
  useSwipe(
    swipeFrom,
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
    enabled,
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
    // Away is off the end: rightward left to right, leftward right to left (`--drawer-away`).
    transform: `translateX(calc(${String(1 - drawn)} * var(--drawer-away)))`,
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
      <Modal
        ref={panel}
        className="absolute inset-y-0 end-0 flex w-[min(20rem,100%_-_3rem)] flex-col border-s border-line bg-surface-raised pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] shadow-xl outline-none [--drawer-away:100%] ltr:pe-[env(safe-area-inset-right)] rtl:pe-[env(safe-area-inset-left)] rtl:[--drawer-away:-100%]"
        style={panelStyle}
      >
        <Dialog className="flex min-h-0 flex-1 flex-col outline-none">
          <div className="px-4 pt-4 pb-2">
            <DialogHeading>{title}</DialogHeading>
          </div>
          {children}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/**
 * How wide a drawer is drawn on a screen this wide, in CSS pixels, as its class has it: 20rem,
 * leaving at least 3rem of the screen beside it. A finger's first move comes before the
 * drawer is drawn to be measured.
 */
function drawerWidth(screen: number): number {
  return Math.min(320, screen - 48);
}
