import { BrowserWindow, app, ipcMain } from "electron";

/**
 * The `aspen://app/…` links invites and sign-in codes are shared as where a deployment names no
 * web client (`shareUrl` in the app). The installers register the scheme with the system
 * (`protocols` in `electron-builder.yml`), and a packaged app claims it again at run time, which
 * an AppImage, having no installer, needs. A development build never claims it, which would
 * hand the system's handler to a bare Electron; it takes links on its command line. One instance of the app runs at a time: a link opened while it
 * runs reaches it, on Windows and Linux as the second launch's command line and on macOS as
 * `open-url`, and a launch that only carried a link hands it over and quits.
 */

const SCHEME = "aspen";
const PREFIX = `${SCHEME}://app/`;
/** Far longer than any link the app shares; anything longer is not one of them. */
const MAX_LINK_CHARS = 2048;

/** Links that arrived before the page asked for them, or while it was loading. */
const pending: string[] = [];
let pageReady = false;

function linkIn(args: readonly string[]): string | undefined {
  return args.find((arg) => arg.startsWith(PREFIX) && arg.length <= MAX_LINK_CHARS);
}

function deliver(url: string): void {
  if (!url.startsWith(PREFIX) || url.length > MAX_LINK_CHARS) {
    return;
  }
  const window = BrowserWindow.getAllWindows()[0];
  if (window !== undefined) {
    if (window.isMinimized()) {
      window.restore();
    }
    window.focus();
  }
  if (pageReady && window !== undefined) {
    window.webContents.send("app-link:open", url);
  } else {
    pending.push(url);
  }
}

/**
 * Claims the scheme and the single instance, before the app is ready. Returns `false` when
 * another instance runs, which has been handed this launch's link; the caller quits.
 */
export function claimAppLinks(): boolean {
  if (!app.requestSingleInstanceLock()) {
    return false;
  }
  if (app.isPackaged) {
    app.setAsDefaultProtocolClient(SCHEME);
  }
  app.on("second-instance", (_event, commandLine) => {
    const url = linkIn(commandLine);
    if (url !== undefined) {
      deliver(url);
    } else {
      const window = BrowserWindow.getAllWindows()[0];
      window?.focus();
    }
  });
  // macOS hands links over as an event, and may do so before the app is ready.
  app.on("open-url", (event, url) => {
    event.preventDefault();
    deliver(url);
  });
  const launched = linkIn(process.argv);
  if (launched !== undefined) {
    pending.push(launched);
  }
  return true;
}

/**
 * The page asks for links once it can open them: it takes those that waited, and later ones
 * come as `app-link:open`. A page that reloads asks again.
 */
export function serveAppLinks(): void {
  ipcMain.handle("app-link:ready", () => {
    pageReady = true;
    return pending.splice(0);
  });
  app.on("browser-window-created", (_event, window) => {
    window.webContents.on("did-start-navigation", (details) => {
      if (details.isMainFrame && !details.isSameDocument) {
        pageReady = false;
      }
    });
  });
}
