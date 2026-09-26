import { normalizeServerUrl } from "@aspen/protocol";

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
  if (window.location.protocol === "capacitor:" || window.location.hostname === "localhost") {
    // Capacitor serves the bundle from `capacitor://localhost` (iOS) or `https://localhost`
    // (Android). A browser tab at `localhost` is the Vite dev server, which also counts as web
    // but needs no server address, so this guess only matters once a build is installed.
    return window.location.protocol === "capacitor:" ? "mobile" : "web";
  }
  return "web";
}

/**
 * The server origin the app should use, in order of preference: what the user last entered,
 * the build-time `VITE_ASPEN_SERVER_URL`, then the page's own origin. Electron and Capacitor
 * have no meaningful own origin, so they always need one of the first two.
 */
export function defaultServerUrl(shell: Shell = detectShell()): string | null {
  try {
    const remembered = window.localStorage.getItem(SERVER_URL_KEY);
    if (remembered !== null) {
      return remembered;
    }
  } catch {
    // Storage may be unavailable (private mode, blocked); fall through.
  }
  const fromEnv = import.meta.env.VITE_ASPEN_SERVER_URL;
  if (fromEnv !== undefined && fromEnv.length > 0) {
    return normalizeServerUrl(fromEnv);
  }
  if (shell === "web" && window.location.protocol.startsWith("http")) {
    return window.location.origin;
  }
  return null;
}

export function rememberServerUrl(serverUrl: string): void {
  try {
    window.localStorage.setItem(SERVER_URL_KEY, serverUrl);
  } catch {
    // Not fatal; the user will be asked again next launch.
  }
}
