/**
 * What the window may show and what it hands to the system. The window only ever shows the app
 * itself; a link the page opens elsewhere goes to the system's handler for its scheme only when
 * that scheme is one of `EXTERNAL_SCHEMES`. Everything else is dropped: `file:` (which a
 * protocol-relative link resolves to on a page loaded from a file, and which names a share on
 * another machine on Windows), and the many schemes the system hands to programs that act on
 * them (`search-ms:`, `ms-officecmd:`, and the like), any of which a message from another
 * deployment could name.
 *
 * This module imports nothing from Electron, so its tests run under plain Node.
 */

/** The schemes a link opened from the page may reach the system's handler with. */
const EXTERNAL_SCHEMES: ReadonlySet<string> = new Set(["http:", "https:", "mailto:"]);

/**
 * The address to hand to `shell.openExternal` for a link the page opened, or `null` when it is
 * not to leave the app: anything but an absolute `http:`, `https:`, or `mailto:` address.
 */
export function externalUrl(url: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  return EXTERNAL_SCHEMES.has(parsed.protocol) ? parsed.href : null;
}

/**
 * Whether the window may navigate to `url`: in development, a page of the dev server's origin;
 * packaged, the app's own `index.html` (whose route is after the `#`), and nothing else, not
 * even another file beside it.
 */
export function isAppPage(url: string, app: { devServerUrl: string } | { indexUrl: string }) {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if ("devServerUrl" in app) {
    return parsed.origin === new URL(app.devServerUrl).origin;
  }
  const index = new URL(app.indexUrl);
  return (
    parsed.protocol === "file:" &&
    index.protocol === "file:" &&
    parsed.host === index.host &&
    parsed.pathname === index.pathname &&
    parsed.search === ""
  );
}
