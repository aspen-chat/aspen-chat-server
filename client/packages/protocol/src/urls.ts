/** Path prefix shared by every REST route and the event stream; mirrors `api::API_PREFIX`. */
export const API_PREFIX = "/api/v1";

/** Whether plain `http:` and `ws:` may reach this machine's own addresses (`allowLoopbackPlainText`). */
let loopbackPlainText = false;

/**
 * Lets the client reach this machine's own addresses over plain `http:` and `ws:`, as the app
 * does in a development build alone, whose dev server and tests run there. Every other build,
 * and every other address, is reached over `https:` and `wss:` only (`securelyReached`).
 */
export function allowLoopbackPlainText(allowed: boolean): void {
  loopbackPlainText = allowed;
}

/** Whether a URL's hostname names this machine: `localhost`, a name under it, or a loopback address. */
export function isLoopbackHost(hostname: string): boolean {
  return (
    hostname === "localhost" ||
    hostname.endsWith(".localhost") ||
    /^127\.\d+\.\d+\.\d+$/.test(hostname) ||
    hostname === "[::1]"
  );
}

/**
 * Whether the client may connect to `url`, or load from it: over `https:` or `wss:`, or over
 * `http:` or `ws:` at this machine's own addresses where `allowLoopbackPlainText` allows it.
 */
export function securelyReached(url: string | URL): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol === "https:" || parsed.protocol === "wss:") {
    return true;
  }
  return (
    (parsed.protocol === "http:" || parsed.protocol === "ws:") &&
    loopbackPlainText &&
    isLoopbackHost(parsed.hostname)
  );
}

/** An address the client refuses to connect to, since it is not reached over HTTPS (`securelyReached`). */
export class InsecureAddressError extends Error {
  readonly address: string;

  constructor(address: string) {
    super(`${address} is not reached over HTTPS`);
    this.name = "InsecureAddressError";
    this.address = address;
  }
}

/** `url`, when the client may connect to it (`securelyReached`); else `InsecureAddressError`. */
export function requireSecure(url: string): string {
  if (!securelyReached(url)) {
    throw new InsecureAddressError(url);
  }
  return url;
}

/**
 * Normalises a user-entered server address to an origin usable as `baseUrl`: adds `https://`
 * when no scheme is given and strips trailing slashes and any path. Throws `TypeError` for what
 * is no web address, and `InsecureAddressError` for one not reached over HTTPS.
 */
export function normalizeServerUrl(input: string): string {
  const trimmed = input.trim();
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  const url = new URL(withScheme);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new TypeError(`unsupported scheme ${url.protocol}`);
  }
  return requireSecure(url.origin);
}

/** WebSocket URL of the event stream for a server origin, over the WebSocket scheme matching its own. */
export function eventStreamUrl(serverUrl: string): string {
  const url = new URL(serverUrl);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = `${API_PREFIX}/events`;
  url.search = "";
  url.hash = "";
  return requireSecure(url.toString());
}
