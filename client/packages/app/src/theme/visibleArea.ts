import { useSyncExternalStore } from "react";
import { TOUCH_ONLY } from "@/features/layout/useMediaQuery";

/**
 * Where the part of the page a person can see lies, as `--visible-top` and `--visible-bottom` on
 * <html>: how far below the page's top it starts, and how far above the page's foot it ends.
 * WebKit on iOS draws its keyboard over the page without making the page shorter (it ignores the
 * viewport's `interactive-widget`) and only scrolls the focused field into view, so a fixed
 * overlay the page's height has its foot, and a modal centred in it its buttons, under the
 * keyboard. Modals' overlays (`overlayClass`) take these insets instead, and centre in what is
 * left. Where the keyboard makes the page shorter, as Chromium's does and the iOS app's web view
 * does (`MainViewController`), both stay 0.
 *
 * It also learns how tall the device's own keyboard is (`useKeyboardHeight`), for the emoji
 * keyboard to take the same room.
 */

/** The insets, in CSS pixels, for a page `pageHeight` tall seen through `viewport`. */
export function visibleInsets(
  pageHeight: number,
  viewport: Pick<VisualViewport, "height" | "offsetTop" | "scale">,
): { top: number; bottom: number } {
  // Pinched in, the visible part is a window the person moves about the page, so an overlay
  // covers the whole page and is seen through it.
  if (Math.abs(viewport.scale - 1) > 0.01) {
    return { top: 0, bottom: 0 };
  }
  const top = Math.max(0, Math.round(viewport.offsetTop));
  return {
    top,
    bottom: Math.max(0, Math.round(pageHeight - viewport.offsetTop - viewport.height)),
  };
}

/**
 * Less than this over the page's foot is not a keyboard: a bar of suggestions alone, or a
 * browser's toolbar sliding away.
 */
const KEYBOARD_MIN_PX = 120;

const STORAGE_KEY = "aspen.keyboardHeight";

/** The keyboard's height in each orientation, which differ. */
type KeyboardHeights = Partial<Record<"portrait" | "landscape", number>>;

/** What `useKeyboardHeight` and `pageHeight` answer, replaced whole when either changes. */
interface Measured {
  /** The page's height with no keyboard over it or making it shorter. */
  pageHeight: number;
  /** The keyboard's last height in the present orientation; `null` until one has been seen. */
  keyboard: number | null;
  /**
   * How much of the page's foot the device's keyboard takes now: 0 without a keyboard, and
   * always 0 off a touch screen. Styles have it as `--keyboard-cover` where the keyboard makes
   * the page shorter, laid out with the change.
   */
  cover: number;
}

/**
 * How much of the page's foot a keyboard takes: what it took off the page's height, where it
 * makes the page shorter, and what it covers, where it is drawn over the page. `null` where that
 * is too little to be a keyboard.
 */
export function keyboardOver(
  fullHeight: number,
  pageHeight: number,
  covered: number,
): number | null {
  const height = fullHeight - pageHeight + covered;
  return height >= KEYBOARD_MIN_PX ? height : null;
}

let full = { width: 0, height: 0 };
let heights: KeyboardHeights = storedHeights();
let measured: Measured = { pageHeight: 0, keyboard: null, cover: 0 };
const listeners = new Set<() => void>();

function storedHeights(): KeyboardHeights {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
    const heights: KeyboardHeights = {};
    for (const turned of ["portrait", "landscape"] as const) {
      const height: unknown =
        typeof stored === "object" && stored !== null && turned in stored
          ? (stored as Record<string, unknown>)[turned]
          : undefined;
      if (typeof height === "number" && height >= KEYBOARD_MIN_PX) {
        heights[turned] = height;
      }
    }
    return heights;
  } catch {
    return {};
  }
}

/** Which way the device is turned, by the page's shape with no keyboard taking from it. */
function orientation(): keyof KeyboardHeights {
  return full.width > full.height ? "landscape" : "portrait";
}

function show(viewport: VisualViewport): void {
  const { top, bottom } = visibleInsets(window.innerHeight, viewport);
  const style = document.documentElement.style;
  style.setProperty("--visible-top", `${top.toString()}px`);
  style.setProperty("--visible-bottom", `${bottom.toString()}px`);
  // A change of width is the device turned or the window resized: its height without a keyboard
  // is found afresh. A keyboard never makes the page taller, so the tallest seen at a width is it.
  if (window.innerWidth !== full.width) {
    full = { width: window.innerWidth, height: window.innerHeight };
  } else {
    full.height = Math.max(full.height, window.innerHeight);
  }
  const turned = orientation();
  // A window made shorter by hand is not a keyboard; only a device typed on screen has one.
  const typedOnScreen = window.matchMedia(TOUCH_ONLY).matches;
  const keyboard = typedOnScreen ? keyboardOver(full.height, window.innerHeight, bottom) : null;
  const cover = typedOnScreen ? Math.max(0, full.height - window.innerHeight + bottom) : 0;
  // Only the page's full height is given to styles, which work out what a keyboard takes from
  // it themselves (`--keyboard-cover`, from the body's height), in the same layout as the page
  // changes: the resize event this runs in comes after the page has been laid out at its new size.
  if (typedOnScreen) {
    style.setProperty("--page-height", `${full.height.toString()}px`);
  } else {
    style.removeProperty("--page-height");
  }
  if (keyboard !== null && heights[turned] !== keyboard) {
    heights = { ...heights, [turned]: keyboard };
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(heights));
    } catch {
      // Not kept; the next launch learns it again the first time the keyboard rises.
    }
  }
  const next = { pageHeight: full.height, keyboard: heights[turned] ?? null, cover };
  if (
    next.pageHeight !== measured.pageHeight ||
    next.keyboard !== measured.keyboard ||
    next.cover !== measured.cover
  ) {
    measured = next;
    for (const listener of listeners) {
      listener();
    }
  }
}

/** Sets the insets and follows them, and the keyboard's height. Called once, as the page starts. */
export function followVisibleArea(): void {
  const viewport = window.visualViewport;
  if (viewport === null) {
    full = { width: window.innerWidth, height: window.innerHeight };
    measured = { pageHeight: full.height, keyboard: null, cover: 0 };
    return;
  }
  const follow = () => {
    show(viewport);
  };
  follow();
  viewport.addEventListener("resize", follow);
  viewport.addEventListener("scroll", follow);
  window.addEventListener("resize", follow);
}

/** The page's height with no keyboard over it or making it shorter, in CSS pixels. */
export function pageHeight(): number {
  return measured.pageHeight || window.innerHeight;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): Measured {
  return measured;
}

/**
 * The device's own keyboard's height in the present orientation, from the screen's foot to the
 * keyboard's top, as it last rose (kept between launches), and the page's height without it;
 * the keyboard's is `null` until it has risen once on this device.
 */
export function useKeyboardHeight(): Measured {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
