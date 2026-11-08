import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
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
 * How long, put away on a touch screen, it keeps its room for the device's keyboard rising in
 * its place before it gives it up anyway: when none rises, as when an attached keyboard types.
 */
const HANDOVER_MS = 1000;

/**
 * The emoji keyboard: the picker below a message box, edge to edge across it (as many whole
 * emoji to a row as fit, with the category bar at the top and the search beside it), as tall as
 * a phone's own keyboard (`useKeyboardHeight`). It stays until it is put away, so several emoji
 * can be picked in a row; each goes to `onPick` as a message names it (a glyph, or a custom
 * emoji's reference). Escape anywhere in it calls `onClose`; the button that opened it closes it
 * too. It is never inside the box's `<form>`: Enter in its search would send the message. As it
 * opens it scrolls into view, for one below a message being edited partway down the list, which
 * the list otherwise leaves where it was.
 *
 * `docked` below the channel's box, it is where the phone's keyboard would be, from the screen's
 * foot, its ground under the home indicator and its emoji above it, and on a touch screen it and
 * the phone's keyboard take each other's place without anything above them moving. While it is
 * there the page keeps its full height (`KeepsPageHeight`) however much of it the phone's
 * keyboard takes, so the keyboard coming or going changes no layout: it covers the emoji
 * keyboard, or leaves it uncovered. Put away (`open` false) as the phone's keyboard rises, its
 * room stays, as tall as that keyboard, for as long as the keyboard is up, and goes once it has
 * gone (or after `HANDOVER_MS` when none comes), when the page's full height is its height
 * anyway. Changing the layout while the web view resizes would not do: WebKit lays the page out
 * at its new size before styles that follow the size are worked out, and keeps the list's
 * position from that layout.
 */
export function EmojiKeyboard({
  id,
  open,
  docked = false,
  communityId,
  onPick,
  onClose,
}: {
  id: string;
  open: boolean;
  docked?: boolean;
  /** The community whose own emoji it offers too; none in a DM. */
  communityId: string | null;
  onPick: (emoji: string) => void;
  onClose: () => void;
}) {
  const m = useMessages();
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  const { pageHeight, keyboard, cover } = useKeyboardHeight();
  const height =
    keyboard ??
    (touchOnly
      ? Math.round(pageHeight * TOUCH_SHARE)
      : Math.min(POINTER_MAX_PX, Math.round(pageHeight * POINTER_SHARE)));
  const [wasOpen, setWasOpen] = useState(open);
  // Put away into the phone's keyboard: `risen` once that keyboard has come.
  const [holding, setHolding] = useState<{ risen: boolean } | null>(null);
  if (open !== wasOpen) {
    setWasOpen(open);
    setHolding(!open && docked && touchOnly ? { risen: false } : null);
  }
  if (holding !== null && !holding.risen && cover > 0) {
    setHolding({ risen: true });
  }
  if (holding?.risen === true && cover === 0) {
    setHolding(null);
  }
  const waiting = holding !== null && !holding.risen;
  useEffect(() => {
    if (!waiting) {
      return;
    }
    const timer = setTimeout(() => {
      setHolding(null);
    }, HANDOVER_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [waiting]);
  const root = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (open) {
      root.current?.scrollIntoView({ block: "nearest" });
    }
  }, [open]);
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    }
  }
  if (!open && holding === null) {
    return null;
  }
  return (
    <>
      {docked && <KeepsPageHeight />}
      {open ? (
        <div
          ref={root}
          id={id}
          role="group"
          aria-label={m.emoji.keyboard}
          onKeyDown={onKeyDown}
          className={
            "bg-surface-raised " +
            (docked
              ? "flex shrink-0 flex-col border-t border-line pb-[env(safe-area-inset-bottom)]"
              : "")
          }
          style={{
            height: docked
              ? `${String(height)}px`
              : `calc(${String(height)}px - env(safe-area-inset-bottom))`,
          }}
        >
          <div className="min-h-0 flex-1" style={docked ? {} : { height: "100%" }}>
            <LazyEmojiPicker communityId={communityId} onPick={onPick} fill height="100%" />
          </div>
        </div>
      ) : (
        // Its room, kept for the phone's keyboard that takes its place, and as tall as that
        // keyboard once it has come.
        <div
          aria-hidden="true"
          className="shrink-0 bg-surface-raised"
          style={{ height: `${String(cover > 0 ? cover : height)}px` }}
        />
      )}
    </>
  );
}

/**
 * Keeps the page its full height while it is drawn (`data-docked-emoji-keyboard` on <html>,
 * `styles.css`), beside the emoji keyboard or the room it keeps, whichever is drawn, so that the
 * page keeps its height as one gives way to the other. A component of its own, so that the mark
 * goes in the same step as the emoji keyboard's elements: React runs the cleanup of a layout
 * effect whose component goes as it takes the component's elements out, before any other effect
 * can read layout between.
 */
function KeepsPageHeight() {
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.dockedEmojiKeyboard = "";
    return () => {
      delete root.dataset.dockedEmojiKeyboard;
    };
  }, []);
  return null;
}
