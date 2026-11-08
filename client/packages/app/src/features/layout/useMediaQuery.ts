import { useCallback, useSyncExternalStore } from "react";

/**
 * Where Tailwind's `md` variant applies (`styles.css`), for layout decisions made in code that
 * must agree with the ones made in class names: 48rem wide, on a screen tall enough for panes
 * side by side or with a mouse or trackpad, so a phone held sideways keeps one pane.
 */
export const MEDIUM_SCREEN = "(min-width: 48rem) and ((min-height: 30rem) or (pointer: fine))";

/**
 * The width alone from which `MEDIUM_SCREEN` applies, however short the screen: a phone held
 * upright is narrower, and held sideways is not.
 */
export const MEDIUM_WIDTH = "(min-width: 48rem)";

/**
 * Where Tailwind's `lg` variant applies, from which the member list is a pane of its own: as
 * `MEDIUM_SCREEN`, from 64rem wide.
 */
export const LARGE_SCREEN = "(min-width: 64rem) and ((min-height: 30rem) or (pointer: fine))";

/**
 * A pointer as coarse as a finger, which Tailwind's `pointer-coarse` variant gives larger
 * targets, for sizes chosen in code that must make room for them.
 */
export const COARSE_POINTER = "(pointer: coarse)";

/**
 * A touch screen among the device's inputs, whether or not it is the main one: a phone, a
 * tablet, or a laptop with one, where a finger may swipe.
 */
export const TOUCH_SCREEN = "(any-pointer: coarse)";

/**
 * A device typed on with an on-screen keyboard: touch alone, nothing to hover with. Its
 * keyboard has no Shift+Enter for a new line, so Enter writes one there and does not send.
 */
export const TOUCH_ONLY = "(hover: none) and (pointer: coarse)";

/**
 * Whether the app shows one pane at a time: a list, or a conversation, never both. Then the
 * pane shown is the page's main content, and a conversation's title is its first heading.
 */
export function useOnePane(): boolean {
  return !useMediaQuery(MEDIUM_SCREEN);
}

/** The query's live result, or `null` where there is no `matchMedia` (tests without a DOM). */
function mediaQueryList(query: string): MediaQueryList | null {
  return typeof window !== "undefined" && "matchMedia" in window ? window.matchMedia(query) : null;
}

/** Whether `query` matches, kept current as the window changes. */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      const list = mediaQueryList(query);
      list?.addEventListener("change", onChange);
      return () => {
        list?.removeEventListener("change", onChange);
      };
    },
    [query],
  );
  return useSyncExternalStore(subscribe, () => mediaQueryList(query)?.matches ?? false);
}
