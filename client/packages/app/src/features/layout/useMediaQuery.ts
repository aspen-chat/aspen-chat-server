import { useCallback, useSyncExternalStore } from "react";

/**
 * The width Tailwind's `md` variant starts at, for layout decisions made in code that must
 * agree with the ones made in class names.
 */
export const MEDIUM_SCREEN = "(min-width: 48rem)";

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
