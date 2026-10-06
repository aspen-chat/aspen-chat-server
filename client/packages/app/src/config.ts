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
 * to, at the deployment's one origin, so it uses the page's own origin and never asks. Electron
 * and Capacitor have no server of their own and bake none in: they use the one the user entered,
 * and until there is one, `null`, which asks (`ServerForm`).
 */
export function defaultServerUrl(shell: Shell = detectShell()): string | null {
  if (shell === "web") {
    return window.location.protocol.startsWith("http") ? window.location.origin : null;
  }
  try {
    return window.localStorage.getItem(SERVER_URL_KEY);
  } catch {
    // Storage may be unavailable (private mode, blocked); the user is asked.
    return null;
  }
}

export function rememberServerUrl(serverUrl: string): void {
  try {
    window.localStorage.setItem(SERVER_URL_KEY, serverUrl);
  } catch {
    // Not fatal; the user will be asked again next launch.
  }
}
