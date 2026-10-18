/**
 * Where the part of the page a person can see lies, as `--visible-top` and `--visible-bottom` on
 * <html>: how far below the page's top it starts, and how far above the page's foot it ends.
 * WebKit on iOS draws its keyboard over the page without making the page shorter (it ignores the
 * viewport's `interactive-widget`) and only scrolls the focused field into view, so a fixed
 * overlay the page's height has its foot, and a modal centred in it its buttons, under the
 * keyboard. Modals' overlays (`overlayClass`) take these insets instead, and centre in what is
 * left. Where the keyboard makes the page shorter, as Chromium's does, both stay 0.
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

function show(viewport: VisualViewport): void {
  const { top, bottom } = visibleInsets(window.innerHeight, viewport);
  const style = document.documentElement.style;
  style.setProperty("--visible-top", `${top.toString()}px`);
  style.setProperty("--visible-bottom", `${bottom.toString()}px`);
}

/** Sets the insets and follows them. Called once, as the page starts. */
export function followVisibleArea(): void {
  const viewport = window.visualViewport;
  if (viewport === null) {
    return;
  }
  const follow = () => {
    show(viewport);
  };
  follow();
  viewport.addEventListener("resize", follow);
  viewport.addEventListener("scroll", follow);
}
