import { app, ipcMain, shell } from "electron";
import { existsSync } from "node:fs";
import { join } from "node:path";

/**
 * Chromium's notices, `LICENSES.chromium.html` from Electron's release, which the Open Source
 * Attributions page cannot carry (they are several megabytes) and so opens in the system's
 * browser. electron-builder copies the file among the resources on every platform
 * (`electron-builder.yml`); unpackaged, it is the installed Electron's.
 */
function noticesFile(): string {
  return app.isPackaged
    ? join(process.resourcesPath, "LICENSES.chromium.html")
    : join(app.getAppPath(), "node_modules", "electron", "dist", "LICENSES.chromium.html");
}

/** Answers `chromium-notices:open` with whether the notices could be opened. */
export function serveChromiumNotices(): void {
  ipcMain.handle("chromium-notices:open", async () => {
    const file = noticesFile();
    if (!existsSync(file)) {
      return false;
    }
    // `openPath` answers with an error message, empty when the file opened.
    return (await shell.openPath(file)) === "";
  });
}
