import { APP_LINK_BASE } from "@/features/qr/shareLinks";

/**
 * Where a link to the app leads, read from the link as a whole: the route it names, and the
 * address it names it on.
 */
export interface RouteAddress {
  /** The route's path, without a slash at its end. */
  readonly path: string;
  readonly search: URLSearchParams;
  /** What follows the route's `#`. */
  readonly fragment: string;
  /**
   * The host the link is on (with its port when it gives one, in lowercase), which is a
   * deployment's domain; `null` for a link on no deployment's address: an `aspen:` link, or a
   * page an app loaded from a file.
   */
  readonly host: string | null;
}

/** A stand-in base for reading a route that has no address of its own. */
const NO_ADDRESS = "https://route.invalid";

function at(route: URL, host: string | null): RouteAddress {
  const path = route.pathname.length > 1 ? route.pathname.replace(/\/$/u, "") : "/";
  return { path, search: route.searchParams, fragment: route.hash.slice(1), host };
}

/** The route a hash-routed page's fragment holds. */
function inFragment(url: URL, host: string | null): RouteAddress | null {
  if (!url.hash.startsWith("#/")) {
    return null;
  }
  try {
    return at(new URL(url.hash.slice(1), NO_ADDRESS), host);
  } catch {
    return null;
  }
}

/**
 * Reads a link to the app in any form one is shared or shown in: under a web client
 * (`https://chat.example.org/invite/abc`), as an `aspen://app/…` link, or as the address of a
 * desktop or mobile app's page, whose route is in its fragment (`…/index.html#/invite/abc`).
 * `null` for text that is not one address, whatever it holds: a route is only ever the whole
 * path of a link, never a part found in its query, another path, or the prose around it.
 */
export function routeAddress(text: string): RouteAddress | null {
  const trimmed = text.trim();
  try {
    // The app's own scheme is read by its prefix: not every engine parses a scheme it does
    // not know into a host and a path.
    if (trimmed.startsWith(`${APP_LINK_BASE}/`)) {
      return at(new URL(trimmed.slice(APP_LINK_BASE.length), NO_ADDRESS), null);
    }
    const url = new URL(trimmed);
    if (url.protocol === "file:") {
      return inFragment(url, null);
    }
    if (url.protocol !== "https:" && url.protocol !== "http:") {
      return null;
    }
    const host = url.host.toLowerCase();
    const page = url.pathname === "/" || url.pathname.endsWith("/index.html");
    return page && url.hash.startsWith("#/") ? inFragment(url, host) : at(url, host);
  } catch {
    return null;
  }
}

/** A deployment's domain as a link writes it, percent-encoded or not; `null` for none. */
export function domainIn(written: string): string | null {
  try {
    const domain = decodeURIComponent(written).trim().toLowerCase();
    return domain === "" ? null : domain;
  } catch {
    return null;
  }
}

const ABROAD = /^\/at\/([^/]+)(\/.*)$/u;

/**
 * The deployment a route is of, and the route within it: under `/at/{domain}`, that
 * deployment's; otherwise the one the link is on, which is `null`, the deployment in use, for a
 * link on one of its own addresses (`homeHosts`) or on none. `null` for an `/at/` with no
 * domain to read.
 */
export function deploymentRoute(
  address: RouteAddress,
  homeHosts: readonly string[],
): { domain: string | null; path: string; named: boolean } | null {
  const abroad = ABROAD.exec(address.path);
  if (abroad !== null) {
    const domain = domainIn(abroad[1] ?? "");
    return domain === null ? null : { domain, path: abroad[2] ?? "/", named: true };
  }
  const { host } = address;
  return {
    domain: host === null || homeHosts.includes(host) ? null : host,
    path: address.path,
    named: false,
  };
}
