import { useLayoutEffect, useRef, type KeyboardEvent } from "react";
import { LazyEmojiPicker } from "@/features/emoji/LazyEmojiPicker";
import { TOUCH_ONLY, useMediaQuery } from "@/features/layout/useMediaQuery";
import { useMessages } from "@/i18n/context";
import { useKeyboardHeight } from "@/theme/visibleArea";

/**
 * Before a touch screen's keyboard has risen, the share of the page the emoji keyboard takes in
 * its place; with a pointer, the share and the most it takes, so it does not tower on a monitor.
 */
const TOUCH_SHARE = 0.4;
const POINTER_SHARE = 0.45;
const POINTER_MAX_PX = 360;

/**
 * The emoji keyboard: the picker docked below a message box, edge to edge across it (as many
 * whole emoji to a row as fit, with the category bar at the top and the search beside it), where
 * a phone's own keyboard would be, and as tall as that keyboard (`useKeyboardHeight`), so the
 * box stays where it was as one takes the other's place. Its height counts from the screen's foot,
 * so the home indicator's room comes off the picker; `Composer` runs the keyboard's ground on under
 * the indicator. It stays until it is put away, so several emoji can be picked in a row; each goes to
 * `onPick` as a message names it (a glyph, or a custom emoji's reference). Escape anywhere in it
 * calls `onClose`; the button that opened it closes it too. It is never inside the box's
 * `<form>`: Enter in its search would send the message. As it opens it scrolls into view, for
 * one below a message being edited partway down the list, which the list otherwise leaves where
 * it was.
 */
export function EmojiKeyboard({
  id,
  communityId,
  onPick,
  onClose,
  className = "",
}: {
  id: string;
  /** The community whose own emoji it offers too; none in a DM. */
  communityId: string | null;
  onPick: (emoji: string) => void;
  onClose: () => void;
  className?: string;
}) {
  const m = useMessages();
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  const { pageHeight, keyboard } = useKeyboardHeight();
  const height =
    keyboard ??
    (touchOnly
      ? Math.round(pageHeight * TOUCH_SHARE)
      : Math.min(POINTER_MAX_PX, Math.round(pageHeight * POINTER_SHARE)));
  const root = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    root.current?.scrollIntoView({ block: "nearest" });
  }, []);
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    }
  }
  return (
    <div
      ref={root}
      id={id}
      role="group"
      aria-label={m.emoji.keyboard}
      onKeyDown={onKeyDown}
      className={"bg-surface-raised " + className}
    >
      <div style={{ height: `calc(${String(height)}px - env(safe-area-inset-bottom))` }}>
        <LazyEmojiPicker communityId={communityId} onPick={onPick} fill height="100%" />
      </div>
    </div>
  );
}
