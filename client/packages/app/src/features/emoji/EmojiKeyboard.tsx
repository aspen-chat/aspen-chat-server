import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { LazyEmojiPicker } from "@/features/emoji/LazyEmojiPicker";
import { pickerHeight } from "@/features/emoji/pickerHeight";
import { useMessages } from "@/i18n/context";

/** The tallest the keyboard is drawn, and the share of the window it may take on a phone. */
const KEYBOARD_MAX_PX = 360;
const KEYBOARD_SHARE = 0.45;

/**
 * The emoji keyboard: the picker docked below a message box, across its width (as many whole
 * emoji to a row as fit, with the category bar at the top and the search beside it), where a
 * phone's own keyboard would be. It stays until it is put away, so several emoji can be
 * picked in a row; each goes to `onPick` as a message names it (a glyph, or a custom emoji's
 * reference). Escape anywhere in it calls `onClose`; the button that opened it closes it too.
 * It is never inside the box's `<form>`: Enter in its search would send the message. As it
 * opens it scrolls into view, for one below a message being edited partway down the list,
 * which the list otherwise leaves where it was.
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
  const height = useKeyboardHeight();
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
      className={"flex justify-center bg-surface-sunken " + className}
    >
      <div className="w-full max-w-3xl overflow-hidden rounded-md border border-line">
        <LazyEmojiPicker communityId={communityId} onPick={onPick} fill height={height} />
      </div>
    </div>
  );
}

/** The keyboard's height, following the window as it is resized or turned. */
function useKeyboardHeight(): number {
  const [height, setHeight] = useState(() => pickerHeight(KEYBOARD_MAX_PX, KEYBOARD_SHARE));
  useEffect(() => {
    const measure = () => {
      setHeight(pickerHeight(KEYBOARD_MAX_PX, KEYBOARD_SHARE));
    };
    window.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("resize", measure);
    };
  }, []);
  return height;
}
