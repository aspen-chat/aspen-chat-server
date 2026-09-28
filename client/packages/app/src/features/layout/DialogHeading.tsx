import { XIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Button, Heading } from "react-aria-components";
import { headingClass } from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";

/**
 * A modal's title, with the X in the top right that closes it (the `Dialog`'s `close` slot,
 * which also tells the modal's owner, as dismissing it would). Every modal uses it except one
 * that must not be left before the reader acts, such as the recovery codes, which ask the
 * reader to confirm they saved them.
 */
export function DialogHeading({ children }: { children: ReactNode }) {
  const m = useMessages();
  return (
    <div className="flex items-start gap-2">
      <Heading slot="title" className={headingClass + " min-w-0 flex-1"}>
        {children}
      </Heading>
      <Tooltip text={m.close}>
        <Button
          slot="close"
          aria-label={m.close}
          className="tap-target -mt-0.5 -mr-1 shrink-0 rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <XIcon size={18} aria-hidden="true" />
        </Button>
      </Tooltip>
    </div>
  );
}
