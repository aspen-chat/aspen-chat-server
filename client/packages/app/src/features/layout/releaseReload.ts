import { detectShell } from "@/config";

/**
 * When the page last reloaded for a new release, kept for the tab's life, so that a release
 * whose files cannot be loaded either is reloaded into once and then left to show its error
 * rather than reloaded without end.
 */
const RELOADED_AT_KEY = "aspen.newRelease.reloadedAt";
const RELOAD_AT_MOST_EVERY_MS = 60_000;

/**
 * Whether this page is a web client of another release than the one its deployment now serves.
 * Only a web client the deployment served can be: the desktop and mobile apps carry their own,
 * and so does the development server.
 */
export function isOutdated(served: string | null): boolean {
  return (
    detectShell() === "web" &&
    __ASPEN_WEB_CLIENT_BUILD__ !== null &&
    served !== null &&
    served !== __ASPEN_WEB_CLIENT_BUILD__
  );
}

/**
 * Reloads into the release the deployment serves, unless this tab did so moments ago. Drafts
 * are kept: the message box saves its draft when the page is hidden, as a reload hides it.
 * Answers whether it reloaded.
 */
export function reloadForNewRelease(): boolean {
  const now = Date.now();
  try {
    const before = Number(window.sessionStorage.getItem(RELOADED_AT_KEY));
    if (Number.isFinite(before) && now - before < RELOAD_AT_MOST_EVERY_MS) {
      return false;
    }
    window.sessionStorage.setItem(RELOADED_AT_KEY, String(now));
  } catch {
    // Without storage there is no telling a loop from a first time; reload anyway.
  }
  window.location.reload();
  return true;
}
