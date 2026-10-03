import { normalizeServerUrl } from "@aspen/protocol";
import { Capacitor } from "@capacitor/core";

const SERVER_URL_KEY = "aspen.serverUrl";

/**
 * Which shell is rendering the app. Drives platform-specific behaviour such as where the
 * session is stored and whether a server address must be asked for.
 */
export type Shell = "web" | "desktop" | "mobile";

export function detectShell(): Shell {
  if (typeof window === "undefined") {
    return "web";
  }
  if (window.aspenDesktop !== undefined) {
    return "desktop";
  }
  // Capacitor serves the bundle from `capacitor://localhost` on iOS and `https://localhost` on
  // Android, which only its own bridge tells apart from a browser tab at `localhost`.
  if (Capacitor.isNativePlatform()) {
    return "mobile";
  }
  return "web";
}

/**
 * The server origin the app should use. The web client is served by the deployment it signs in
 * to, so it uses the page's own origin, or the build-time `VITE_ASPEN_SERVER_URL` when one is
 * set (a development build pointed elsewhere); it never asks. Electron and Capacitor have no
 * meaningful own origin, so they use what the user last entered, then the build-time address,
 * and otherwise ask.
 */
export function defaultServerUrl(shell: Shell = detectShell()): string | null {
  const fromEnv = import.meta.env.VITE_ASPEN_SERVER_URL;
  const built = fromEnv !== undefined && fromEnv.length > 0 ? normalizeServerUrl(fromEnv) : null;
  if (shell === "web") {
    return built ?? (window.location.protocol.startsWith("http") ? window.location.origin : null);
  }
  try {
    const remembered = window.localStorage.getItem(SERVER_URL_KEY);
    if (remembered !== null) {
      return remembered;
    }
  } catch {
    // Storage may be unavailable (private mode, blocked); fall through.
  }
  return built;
}

export function rememberServerUrl(serverUrl: string): void {
  try {
    window.localStorage.setItem(SERVER_URL_KEY, serverUrl);
  } catch {
    // Not fatal; the user will be asked again next launch.
  }
}
