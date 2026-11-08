import { ArrowLeftIcon, XIcon } from "@phosphor-icons/react";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { Button, Heading } from "react-aria-components";
import { headingClass } from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";

const iconButtonClass =
  "tap-target -mt-0.5 shrink-0 rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * A modal's title row: the title, with the X in the top right that closes it (the `Dialog`'s
 * `close` slot, which also tells the modal's owner, as dismissing it would), and on a later step
 * of a stepped dialog an arrow before the title that returns to the choice (`onBack`). The row
 * lays out every control a title has, so nothing is placed beside it from outside, where the
 * title would no longer take the width that keeps the X in the corner. Every modal uses it,
 * and in a modal it stays at the top of the frame while the modal scrolls (`pinned-title`), so
 * the X is always in reach. The frame scrolls a control it brings into view, such as one that
 * takes focus, to below the row rather than under it: the row keeps its height in the frame's
 * `--pinned-title-height`, which the frame's `scroll-padding-top` reads.
 * Without `closeButton` it has no X, for a modal that must not be left before the reader acts,
 * such as the recovery codes, which ask the reader to confirm they saved them, or whose own
 * buttons are the only ways out, as an incoming call's Accept and Decline are.
 */
export function DialogHeading({
  children,
  closeButton = true,
  onBack,
}: {
  children: ReactNode;
  closeButton?: boolean;
  onBack?: () => void;
}) {
  const m = useMessages();
  const row = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = row.current;
    const frame = element?.closest<HTMLElement>(".modal-frame");
    // Without `ResizeObserver` (jsdom) the frame keeps its `scroll-padding-top` fallback.
    if (!element || !frame || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(() => {
      frame.style.setProperty("--pinned-title-height", `${String(element.offsetHeight)}px`);
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
      frame.style.removeProperty("--pinned-title-height");
    };
  }, []);
  return (
    <div ref={row} className="pinned-title flex items-start gap-2">
      {onBack && (
        <Tooltip text={m.back}>
          <Button onPress={onBack} aria-label={m.back} className={iconButtonClass + " -ms-1"}>
            <ArrowLeftIcon size={18} aria-hidden="true" className="rtl:-scale-x-100" />
          </Button>
        </Tooltip>
      )}
      <Heading slot="title" className={headingClass + " min-w-0 flex-1"}>
        {children}
      </Heading>
      {closeButton && (
        <Tooltip text={m.close}>
          <Button slot="close" aria-label={m.close} className={iconButtonClass + " -me-1"}>
            <XIcon size={18} aria-hidden="true" />
          </Button>
        </Tooltip>
      )}
    </div>
  );
}
