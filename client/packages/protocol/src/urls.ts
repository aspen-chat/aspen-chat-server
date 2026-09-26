/** Path prefix shared by every REST route and the event stream; mirrors `api::API_PREFIX`. */
export const API_PREFIX = "/api/v1";

/**
 * Normalises a user-entered server address to an origin usable as `baseUrl`: adds `https://`
 * when no scheme is given and strips trailing slashes and any path.
 */
export function normalizeServerUrl(input: string): string {
  const trimmed = input.trim();
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  const url = new URL(withScheme);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new TypeError(`unsupported scheme ${url.protocol}`);
  }
  return url.origin;
}

/** WebSocket URL of the event stream for a server origin. */
export function eventStreamUrl(serverUrl: string): string {
  const url = new URL(serverUrl);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = `${API_PREFIX}/events`;
  url.search = "";
  url.hash = "";
  return url.toString();
}
