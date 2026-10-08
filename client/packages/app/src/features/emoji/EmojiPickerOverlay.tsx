import { SmileyIcon } from "@phosphor-icons/react";
import { lazy, Suspense, useRef, useState, type ReactNode } from "react";
import { Button, Dialog, Popover, type PopoverProps } from "react-aria-components";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { Drawer } from "@/features/layout/Drawer";
import { TOUCH_ONLY, useMediaQuery } from "@/features/layout/useMediaQuery";
import { useMessages } from "@/i18n/context";

/** The emoji picker is a sizeable chunk, fetched the first time anyone opens it. */
const EmojiPicker = lazy(() => import("@/features/messages/EmojiPicker"));

/** The tallest the picker is drawn in a sheet, and the share of the screen it may take. */
const SHEET_PICKER_MAX_PX = 440;
const SHEET_PICKER_SHARE = 0.6;

/**
 * The emoji picker, named `label`, wherever an emoji is chosen: on a touch screen a sheet
 * sliding up from the bottom (`Drawer`), the picker across its width, which a finger pulls
 * down to put away; elsewhere a popover by `popover`'s trigger. Each pick goes to `onPick`,
 * which closes it through `popover.onOpenChange` when it is done with it; `children` follow
 * the picker (an error, a button that clears the choice).
 */
export function EmojiPickerOverlay({
  label,
  communityId,
  onPick,
  popover,
  children,
}: {
  label: string;
  /** The community whose own emoji the picker offers too; none outside one. */
  communityId: string | null;
  onPick: (emoji: string) => void;
  popover: Omit<PopoverProps, "children" | "className" | "isOpen" | "onOpenChange"> & {
    isOpen: boolean;
    onOpenChange: (open: boolean) => void;
  };
  children?: ReactNode;
}) {
  const m = useMessages();
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  if (touchOnly) {
    const height = Math.min(
      SHEET_PICKER_MAX_PX,
      Math.round(window.innerHeight * SHEET_PICKER_SHARE),
    );
    return (
      <Drawer
        edge="bottom"
        isOpen={popover.isOpen}
        onOpenChange={popover.onOpenChange}
        title={label}
      >
        <div className="flex min-h-0 flex-col overflow-y-auto overscroll-contain pb-2">
          <Suspense
            fallback={
              <div
                className="flex items-center justify-center text-sm text-ink-muted"
                style={{ height }}
              >
                {m.loading}
              </div>
            }
          >
            <EmojiPicker communityId={communityId} onPick={onPick} fill height={height} />
          </Suspense>
          {children}
        </div>
      </Drawer>
    );
  }
  return (
    <Popover {...popover} className="rounded-lg border border-line bg-surface-raised shadow-lg">
      <Dialog aria-label={label} className="outline-none">
        <div className="flex flex-col">
          <Suspense
            fallback={
              <div className="flex h-96 w-80 items-center justify-center text-sm text-ink-muted">
                {m.loading}
              </div>
            }
          >
            <EmojiPicker communityId={communityId} onPick={onPick} />
          </Suspense>
          {children}
        </div>
      </Dialog>
    </Popover>
  );
}

/**
 * A control that chooses one emoji, or none (a poll option's, a status's): it shows the chosen
 * emoji, or a smiley when there is none, and opens the picker, which offers to clear a choice
 * (`clearLabel`). Picking the chosen emoji again clears it too.
 */
export function EmojiChoiceButton({
  label,
  clearLabel,
  emoji,
  onChange,
  className,
}: {
  label: string;
  clearLabel: string;
  emoji: string | null;
  onChange: (emoji: string | null) => void;
  className: string;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const choose = (next: string | null) => {
    onChange(next);
    setOpen(false);
  };
  return (
    <>
      <Button
        ref={trigger}
        aria-label={label}
        onPress={() => {
          setOpen(true);
        }}
        className={className}
      >
        {emoji ?? <SmileyIcon size={18} aria-hidden="true" />}
      </Button>
      <EmojiPickerOverlay
        label={label}
        communityId={null}
        onPick={(picked) => {
          choose(picked === emoji ? null : picked);
        }}
        popover={{
          triggerRef: trigger,
          placement: "bottom start",
          isOpen: open,
          onOpenChange: setOpen,
        }}
      >
        {emoji !== null && (
          <Button
            onPress={() => {
              choose(null);
            }}
            className={secondaryButtonClass + " m-2"}
          >
            {clearLabel}
          </Button>
        )}
      </EmojiPickerOverlay>
    </>
  );
}
