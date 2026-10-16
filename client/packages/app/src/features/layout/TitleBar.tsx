import { useEffect, useRef } from "react";
import { useTitle } from "@/features/layout/documentTitle";
import { hexColor, useTitleBarShown } from "@/theme/titleBar";

/**
 * The window's title bar where the desktop app draws its own (`theme/titleBar.ts`): a strip
 * across the top in the community rail's colour, holding the page's title, by which the window is
 * dragged, and ruled off from the page below in the colour of its dividing lines. The system
 * draws its buttons over one end; the title is centred in the rest.
 */
export function TitleBar() {
  return useTitleBarShown() ? <TitleStrip /> : null;
}

function TitleStrip() {
  const title = useTitle();
  const strip = useRef<HTMLDivElement>(null);
  usePaintWindowButtons(strip);
  return (
    // The title is the document's, which assistive technology already reads.
    <div
      ref={strip}
      aria-hidden="true"
      className="fixed inset-x-0 top-0 h-(--title-bar-height) border-b border-line bg-surface-rail text-ink [-webkit-app-region:drag]"
    >
      {/* The area beside the buttons is given in physical terms, from the window's left edge,
          whichever way the page is written. */}
      <div className="absolute inset-y-0 left-[env(titlebar-area-x,0px)] flex w-[env(titlebar-area-width,100%)] items-center justify-center px-2">
        <span className="truncate text-xs text-ink-muted">{title}</span>
      </div>
    </div>
  );
}

/**
 * Gives the window buttons the strip's colours: its ground, and its ink for their symbols. They
 * follow the palette, the light or dark mode (which may follow the system's), more contrast, and
 * forced colours, so each is watched, and whatever the strip's colours then come to is sent if
 * it changed.
 */
function usePaintWindowButtons(strip: React.RefObject<HTMLDivElement | null>): void {
  useEffect(() => {
    const desktop = window.aspenDesktop?.titleBar;
    const element = strip.current;
    if (desktop === undefined || element === null) {
      return;
    }
    let sent = "";
    const paint = () => {
      const style = getComputedStyle(element);
      const color = hexColor(style.backgroundColor);
      const symbolColor = hexColor(style.color);
      if (color === null || symbolColor === null || sent === color + symbolColor) {
        return;
      }
      sent = color + symbolColor;
      desktop.paint({ color, symbolColor });
    };
    paint();
    const root = new MutationObserver(paint);
    root.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme", "data-contrast", "style"],
    });
    // Only heard as a moment to read the colours again; what they are is the strip's to say.
    const queries = ["(prefers-color-scheme: dark)", "(forced-colors: active)"].map((query) =>
      window.matchMedia(query),
    );
    for (const query of queries) {
      query.addEventListener("change", paint);
    }
    return () => {
      root.disconnect();
      for (const query of queries) {
        query.removeEventListener("change", paint);
      }
    };
  }, [strip]);
}
