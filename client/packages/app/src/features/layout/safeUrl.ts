/**
 * Addresses that reach the page from elsewhere (a message's text, and every URL a deployment
 * sends: a link preview's, an attachment's, an icon's, a plugin's card or annotation), checked
 * before they become a link, a picture, or a window. Any deployment the user holds a session on
 * writes some of them, so none is trusted to name a scheme that is safe here: `javascript:` and
 * `data:` run in or replace the page, `file:` (which a protocol-relative or relative address
 * becomes on a page loaded from a file, as the desktop and mobile apps' are) reads the reader's
 * disk or a share on another machine, and the system hands other schemes to programs that act on
 * them. Each function takes an absolute address only, never resolving one against the page.
 */

import { securelyReached } from "@aspen/protocol";

/** The address parsed, when it is absolute and has one of `schemes`. */
function withScheme(url: string | null | undefined, schemes: readonly string[]): URL | null {
  if (url == null) {
    return null;
  }
  let parsed: URL;
  try {
    // No base: a relative or protocol-relative address does not parse.
    parsed = new URL(url);
  } catch {
    return null;
  }
  return schemes.includes(parsed.protocol) ? parsed : null;
}

/**
 * A web page to open, from a link someone shared or a deployment sent (a link preview's page, a
 * plugin's card or annotation): `http:` or `https:`, or `undefined` for anything else. Plain
 * `http:` is allowed because the pages people link to may be served that way, and opening one
 * runs nothing here.
 */
export function webPageUrl(url: string | null | undefined): string | undefined {
  return withScheme(url, ["https:", "http:"])?.href;
}

/**
 * The target of a link in a message's text: a web page as `webPageUrl` allows, or a `mailto:`
 * address; `undefined` for anything else, a relative address included.
 */
export function messageLinkUrl(url: string | null | undefined): string | undefined {
  return withScheme(url, ["https:", "http:", "mailto:"])?.href;
}

/**
 * A file a deployment serves (a picture, a video, an attachment to download, an icon): `https:`,
 * or `http:` at this machine's own addresses in a development build (`securelyReached`);
 * `undefined` for anything else.
 */
export function mediaUrl(url: string | null | undefined): string | undefined {
  const parsed = withScheme(url, ["https:", "http:"]);
  return parsed !== null && securelyReached(parsed) ? parsed.href : undefined;
}

/**
 * A `mailto:` link to `address` alone: everything but its `@` is percent-encoded, so an address
 * a deployment sent cannot add a subject, a body, or more recipients (`?`, `&`, `,`) to the mail
 * it opens.
 */
export function mailtoUrl(address: string): string {
  return `mailto:${encodeURIComponent(address).replaceAll("%40", "@")}`;
}
