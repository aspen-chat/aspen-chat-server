/**
 * The system's text size, followed where the platform has one a web page can read and the app
 * would otherwise ignore: iOS and iPadOS, whose web views draw pages at a fixed size whatever
 * Larger Text says, unless the page asks for the system's body text (`-apple-system-body`).
 * Android's web view follows its Font size on its own, and desktops' scaling reaches the page
 * as browsers' does. The ratio of the body text's size to its size at the default setting is
 * written on <html> as `--system-text-scale`, which every type size multiplies (`type-scale` in
 * `styles.css`), so text grows as in the system's own apps while spacing and icons keep theirs.
 */

/** The system's body text at its default setting ("Large"), in CSS pixels. */
export const DEFAULT_BODY_PX = 17;

/** How much larger than normal the app's text is drawn for a system body text of `px`. */
export function systemTextScale(px: number): number {
  return Number.isFinite(px) && px > 0 ? px / DEFAULT_BODY_PX : 1;
}

/** Whether this is an iPhone or iPad, whose iPadOS Safari names itself a Mac with a touch screen. */
export function isAppleTouch(userAgent: string, touchPoints: number): boolean {
  return /iPhone|iPad|iPod/.test(userAgent) || (userAgent.includes("Macintosh") && touchPoints > 1);
}

/**
 * Follows the system's text size from now on, where there is one to follow: a hidden probe set
 * in the system's body text is measured, and again whenever its size changes, which it does
 * when the setting changes while the app is open. Called once, as the page starts.
 */
export function followSystemTextSize(): void {
  if (
    !isAppleTouch(navigator.userAgent, navigator.maxTouchPoints) ||
    !CSS.supports("font", "-apple-system-body")
  ) {
    return;
  }
  const probe = document.createElement("span");
  probe.setAttribute("aria-hidden", "true");
  probe.textContent = "M";
  probe.style.cssText =
    "font: -apple-system-body; position: absolute; visibility: hidden; pointer-events: none;";
  document.body.append(probe);
  const follow = () => {
    document.documentElement.style.setProperty(
      "--system-text-scale",
      String(systemTextScale(parseFloat(getComputedStyle(probe).fontSize))),
    );
  };
  follow();
  new ResizeObserver(follow).observe(probe);
}
