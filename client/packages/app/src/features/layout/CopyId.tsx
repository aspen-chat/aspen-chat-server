import { CheckIcon, IdentificationCardIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { Button, MenuItem } from "react-aria-components";
import { useIdWizard } from "@/api/hooks";
import { copyText } from "@/features/layout/clipboard";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";

/** What an id belongs to, as the ID wizard names it: "Copy message ID". */
export type IdThing = keyof Messages["bots"]["idOf"];

/** How long a copy button says it copied before it offers to copy again. */
const COPIED_MS = 1500;

/**
 * The ID wizard's control for one record: copies its id, and says for a moment that it did.
 * It shows only while the wizard is on (`useIdWizard`), and goes last in whatever holds it, so
 * the controls people use every day keep their places whether it is on or off.
 */
export function CopyIdButton({
  id,
  thing,
  className = "",
}: {
  id: string;
  thing: IdThing;
  /** Sizes it to sit among the controls beside it. */
  className?: string;
}) {
  const m = useMessages();
  const on = useIdWizard();
  const button = useRef<HTMLButtonElement>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) {
      return;
    }
    const timer = setTimeout(() => {
      setCopied(false);
    }, COPIED_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [copied]);
  if (!on) {
    return null;
  }
  const name = m.bots.idOf[thing];
  const label = format(copied ? m.bots.copiedId : m.bots.copyId, { thing: name });
  return (
    <Tooltip text={label}>
      <Button
        ref={button}
        aria-label={label}
        onPress={() => {
          if (button.current !== null) {
            void copyText(id, button.current).then(setCopied);
          }
        }}
        className={
          "rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
          className
        }
      >
        {copied ? (
          <CheckIcon size={16} aria-hidden="true" />
        ) : (
          <IdentificationCardIcon size={16} aria-hidden="true" />
        )}
      </Button>
    </Tooltip>
  );
}

/**
 * The ID wizard's entry at the end of a menu: copies the record's id. It shows only while the
 * wizard is on; a menu closes when an entry is chosen, so it says nothing more.
 */
export function CopyIdMenuItem({
  id,
  thing,
  className,
}: {
  id: string;
  thing: IdThing;
  className: string;
}) {
  const m = useMessages();
  const on = useIdWizard();
  if (!on) {
    return null;
  }
  return (
    <MenuItem
      id="copy-id"
      onAction={() => {
        void copyText(id, document.activeElement ?? document.body);
      }}
      className={className + " flex items-center gap-2"}
    >
      <IdentificationCardIcon size={14} aria-hidden="true" className="shrink-0 text-ink-muted" />
      {format(m.bots.copyId, { thing: m.bots.idOf[thing] })}
    </MenuItem>
  );
}
