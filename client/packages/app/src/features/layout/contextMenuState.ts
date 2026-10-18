import {
  useCallback,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type RefObject,
} from "react";

/** How React Aria marks the layer it lays over the page behind a modal popover. */
export const UNDERLAY = "underlay";

/** How soon after the Menu key or Shift+F10 goes down the menu it asks for comes. */
const MENU_KEY_MS = 1000;

/** When the Menu key or Shift+F10 last went down, as `performance.now()` counts. */
let menuKeyAt = -Infinity;

/** Whether `event` is a key that asks for a context menu. */
function isMenuKey(event: KeyboardEvent): boolean {
  return event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
}

if (typeof document !== "undefined") {
  document.addEventListener(
    "keydown",
    (event) => {
      if (isMenuKey(event)) {
        menuKeyAt = performance.now();
      }
    },
    true,
  );
}

/** A point on the screen, in the viewport's pixels. */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** What a `contextmenu` event says about where it came from, as `wantsNativeMenu` reads it. */
export interface ContextMenuCause {
  readonly shiftKey: boolean;
  readonly defaultPrevented: boolean;
  readonly target: EventTarget | null;
  readonly currentTarget: Element;
  /** The pointer that asked, where the browser says: `touch` for a finger held down. */
  readonly pointerType?: string;
}

/**
 * Whether a right click should get the browser's own menu rather than the app's: with Shift
 * held, which is how to reach the browser's menu anywhere; in a text field, whose own menu has
 * the spelling suggestions, cut, and paste; on a playing video or sound, whose own menu controls
 * it; and over text the reader has selected inside what was clicked, which they are about to
 * copy or look up. A finger held down is left to the touch screen's own long press, which
 * offers these menus' actions in sheets (`MessageActionSheet`) or by buttons. An event that
 * something inside already took (`defaultPrevented`), as a user's name inside a message takes
 * it, is that one's menu and not this one's either.
 */
export function wantsNativeMenu(cause: ContextMenuCause, selection: Selection | null): boolean {
  if (cause.defaultPrevented || cause.shiftKey || cause.pointerType === "touch") {
    return true;
  }
  const target = cause.target;
  if (
    target instanceof Element &&
    target.closest(
      "input, textarea, select, video, audio, [contenteditable]:not([contenteditable='false'])",
    ) !== null
  ) {
    return true;
  }
  if (selection === null || selection.isCollapsed || selection.rangeCount === 0) {
    return false;
  }
  for (let i = 0; i < selection.rangeCount; i++) {
    if (selection.getRangeAt(i).intersectsNode(cause.currentTarget)) {
      return true;
    }
  }
  return false;
}

/**
 * Takes a right click for the app, keeping the browser's menu from opening, unless that menu is
 * the one wanted (`wantsNativeMenu`); says whether it took it. For what a right click does
 * other than open a menu of the app's, such as a community's name opening its settings.
 */
export function claimContextMenu(event: ReactMouseEvent): boolean {
  const native = event.nativeEvent;
  const cause: ContextMenuCause = {
    shiftKey: event.shiftKey,
    defaultPrevented: event.defaultPrevented,
    target: event.target,
    currentTarget: event.currentTarget,
    ...("pointerType" in native ? { pointerType: (native as PointerEvent).pointerType } : {}),
  };
  if (wantsNativeMenu(cause, window.getSelection())) {
    return false;
  }
  event.preventDefault();
  return true;
}

/**
 * Where a menu opened by `event` goes: at the pointer, or, opened from the keyboard (the Menu
 * key, Shift+F10), which gives no pointer of its own, under the start of what had focus.
 */
function pointOf(event: ReactMouseEvent): { point: Point; byKeyboard: boolean } {
  const native = event.nativeEvent;
  const pointerType = "pointerType" in native ? (native as PointerEvent).pointerType : undefined;
  // Browsers mark the keyboard's own differently, if at all: Chromium names no button (-1),
  // others no pointer, or no point; the key itself was noted as it went down.
  const byKeyboard =
    performance.now() - menuKeyAt < MENU_KEY_MS ||
    event.button === -1 ||
    pointerType === "" ||
    (event.clientX === 0 && event.clientY === 0);
  if (!byKeyboard) {
    return { point: { x: event.clientX, y: event.clientY }, byKeyboard };
  }
  const from = event.target instanceof Element ? event.target : event.currentTarget;
  const box = from.getBoundingClientRect();
  const rtl = getComputedStyle(from).direction === "rtl";
  return { point: { x: rtl ? box.right : box.left, y: box.bottom }, byKeyboard };
}

/** One context menu's state, from `useContextMenu`. */
export interface ContextMenuState {
  readonly isOpen: boolean;
  /** Whether it opened at a point (a right click, the Menu key) or by a button of its own. */
  readonly from: "point" | "button";
  /**
   * Whether it has opened, from when the menu and what it opens are built: they stay built so
   * each can move as it closes, and what an item opens outlives the menu.
   */
  readonly opened: boolean;
  /** Whether the keyboard opened it, when the menu's first item takes focus. */
  readonly byKeyboard: boolean;
  /** Where it opened, which `anchorRef` stands at; null before it has. */
  readonly point: Point | null;
  /** An element at `point`, which the menu, and anything an item opens, is placed against. */
  readonly anchorRef: RefObject<HTMLSpanElement | null>;
  readonly onOpenChange: (open: boolean) => void;
  /**
   * The `contextmenu` handler of what the menu is for. It opens the menu at the pointer, or
   * leaves the event alone where the browser's own menu is wanted (`wantsNativeMenu`), and says
   * which it did. `before` runs as the menu opens, to read what it will need.
   */
  readonly onContextMenu: (event: ReactMouseEvent, before?: () => void) => boolean;
  /**
   * Opens the menu by its button, beside the button, or closes it if it is open. Pressed from
   * the keyboard (`byKeyboard`), its first item takes focus.
   */
  readonly toggleFromButton: (byKeyboard: boolean) => void;
}

/**
 * A context menu's state, opened by right-clicking what it is for (`onContextMenu`), or by a
 * button of its own for those who do not right-click (`toggleFromButton`). Drawn by
 * `ContextMenu`, or by a menu's own popover placed against `anchorRef` while it opened at a
 * point, with `MenuPoint` beside it.
 */
export function useContextMenu(): ContextMenuState {
  const [isOpen, setOpen] = useState(false);
  const [from, setFrom] = useState<"point" | "button">("point");
  const [opened, setOpened] = useState(false);
  const [byKeyboard, setByKeyboard] = useState(false);
  const [point, setPoint] = useState<Point | null>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const onContextMenu = useCallback((event: ReactMouseEvent, before?: () => void) => {
    if (!claimContextMenu(event)) {
      return false;
    }
    const where = pointOf(event);
    before?.();
    setFrom("point");
    setPoint(where.point);
    setByKeyboard(where.byKeyboard);
    setOpened(true);
    setOpen(true);
    return true;
  }, []);
  const toggleFromButton = useCallback((keyboard: boolean) => {
    setFrom("button");
    setByKeyboard(keyboard);
    setOpened(true);
    setOpen((open) => !open);
  }, []);
  return {
    isOpen,
    from,
    opened,
    byKeyboard,
    point,
    anchorRef,
    onOpenChange: setOpen,
    onContextMenu,
    toggleFromButton,
  };
}
