import { useCallback, useSyncExternalStore } from "react";

/**
 * The width Tailwind's `md` variant starts at, for layout decisions made in code that must
 * agree with the ones made in class names.
 */
export const MEDIUM_SCREEN = "(min-width: 48rem)";

/** The width Tailwind's `lg` variant starts at, from which the member list is a pane of its own. */
export const LARGE_SCREEN = "(min-width: 64rem)";

/**
 * A pointer as coarse as a finger, which Tailwind's `pointer-coarse` variant gives larger
 * targets, for sizes chosen in code that must make room for them.
 */
export const COARSE_POINTER = "(pointer: coarse)";

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
