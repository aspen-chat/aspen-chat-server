import { BrowserWindow, app, desktopCapturer, session, shell } from "electron";
import { join } from "node:path";

/** Where `pnpm dev` serves the app; override with `ASPEN_DEV_SERVER_URL`. */
const devServerUrl = process.env.ASPEN_DEV_SERVER_URL ?? "http://localhost:5173";

function isDevelopment(): boolean {
  return !app.isPackaged;
}

function createWindow(): void {
  const window = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 480,
    minHeight: 400,
    show: false,
    autoHideMenuBar: true,
    webPreferences: {
      preload: join(import.meta.dirname, "../preload/index.cjs"),
      // The renderer is an ordinary web page. It gets no Node access; anything it needs from
      // the host goes through the preload bridge.
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });

  window.once("ready-to-show", () => {
    window.show();
  });

  // Links to other sites open in the user's browser; the window only ever shows the app.
  window.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: "deny" };
  });
  window.webContents.on("will-navigate", (event, url) => {
    const allowed = isDevelopment() ? url.startsWith(devServerUrl) : url.startsWith("file:");
    if (!allowed) {
      event.preventDefault();
      void shell.openExternal(url);
    }
  });

  if (isDevelopment()) {
    void window.loadURL(devServerUrl);
  } else {
    void window.loadFile(join(process.resourcesPath, "app", "index.html"));
  }
}

/**
 * Screen sharing. A page's `getDisplayMedia` does nothing in Electron until the main process
 * answers it. Where the platform has its own picker (macOS 15 and later) that is used; elsewhere
 * the primary screen is shared, with the system's audio on Windows, which is the whole-screen
 * share the app offers today. A picker of windows and screens is a later refinement.
 */
function serveDisplayMedia(): void {
  session.defaultSession.setDisplayMediaRequestHandler(
    (_request, callback) => {
      desktopCapturer
        .getSources({ types: ["screen"] })
        .then((sources) => {
          const [screen] = sources;
          if (screen === undefined) {
            callback({});
            return;
          }
          callback(
            process.platform === "win32" ? { video: screen, audio: "loopback" } : { video: screen },
          );
        })
        .catch(() => {
          callback({});
        });
    },
    { useSystemPicker: true },
  );
}

app
  .whenReady()
  .then(() => {
    serveDisplayMedia();
    createWindow();
    app.on("activate", () => {
      if (BrowserWindow.getAllWindows().length === 0) {
        createWindow();
      }
    });
  })
  .catch((error: unknown) => {
    console.error(error);
    app.quit();
  });

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});
