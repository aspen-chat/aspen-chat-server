import { ArrowLeftIcon, XIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
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
 * title would no longer take the width that keeps the X in the corner. Every modal uses it.
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
  return (
    <div className="flex items-start gap-2">
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
