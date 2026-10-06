import { BrowserWindow, app, desktopCapturer, ipcMain, session, shell } from "electron";
import { join } from "node:path";
import { claimAppLinks, serveAppLinks } from "./appLinks";
import { serveChromiumNotices } from "./chromiumNotices";
import { serveGameCapture } from "./gameCapture";
import { servePasskeyHandoff } from "./passkeyHandoff";
import { serveZoom, storedZoom, zoomWindow } from "./zoom";

/** Where `pnpm dev` serves the app; override with `ASPEN_DEV_SERVER_URL`. */
const devServerUrl = process.env.ASPEN_DEV_SERVER_URL ?? "http://localhost:5173";

/**
 * Automation switches, read by the end-to-end drives that run the shell headlessly: keep the
 * window hidden, and use Chromium's fake camera and microphone so a call can be joined without
 * hardware.
 */
const hidden = process.env.ASPEN_DESKTOP_HIDDEN === "1";
if (process.env.ASPEN_DESKTOP_USER_DATA !== undefined) {
  // A drive starts from a clean profile: no remembered server, session, or permissions.
  app.setPath("userData", process.env.ASPEN_DESKTOP_USER_DATA);
}
if (process.env.ASPEN_DESKTOP_FAKE_MEDIA === "1") {
  app.commandLine.appendSwitch("use-fake-device-for-media-stream");
  app.commandLine.appendSwitch("use-fake-ui-for-media-stream");
  app.commandLine.appendSwitch("autoplay-policy", "no-user-gesture-required");
}

// A second launch that only carried a link has handed it to the running app.
const firstInstance = claimAppLinks();
if (!firstInstance) {
  app.quit();
}

function isDevelopment(): boolean {
  return !app.isPackaged;
}

/**
 * The window's icon on Linux, where each window names its own; Windows and macOS take the
 * application's.
 */
function windowIcon(): string | undefined {
  if (process.platform !== "linux") {
    return undefined;
  }
  return app.isPackaged
    ? join(process.resourcesPath, "icon.png")
    : join(app.getAppPath(), "build", "icons", "512x512.png");
}

function createWindow(): void {
  const icon = windowIcon();
  const window = new BrowserWindow({
    ...(icon === undefined ? {} : { icon }),
    width: 1280,
    height: 800,
    minWidth: 480,
    minHeight: 400,
    show: false,
    autoHideMenuBar: true,
    webPreferences: {
      preload: join(import.meta.dirname, "../preload/index.cjs"),
      // A hidden window must keep decoding and sending media for the drives that use it.
      backgroundThrottling: !hidden,
      // The renderer is an ordinary web page. It gets no Node access; anything it needs from
      // the host goes through the preload bridge.
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      zoomFactor: storedZoom(),
    },
  });
  zoomWindow(window);

  window.once("ready-to-show", () => {
    if (!hidden) {
      window.show();
    }
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
 * answers it with a source. Where the platform has its own picker it is used: macOS 15 and
 * later through `useSystemPicker`, and Wayland through the desktop portal, whose dialog opens
 * when the sources are asked for and offers exactly the kinds asked for, so both screens and
 * windows are requested and the one the user picked is what comes back. Everywhere else
 * Electron lists every screen and window with a thumbnail, and the renderer shows them in a
 * picker of the app's own (`SourcePickerDialog`); its choice, and whether to take the system's
 * audio along on Windows, come back over IPC.
 */
const SYSTEM_PICKER = process.platform === "linux" && process.env.XDG_SESSION_TYPE === "wayland";

interface SourceChoice {
  id: string | null;
  systemAudio: boolean;
}

function serveDisplayMedia(): void {
  session.defaultSession.setDisplayMediaRequestHandler(
    (request, callback) => {
      // Denying a video request with `{}` makes some Electron builds throw
      // "Video was requested, but no video stream was provided"; that throw must not escape,
      // or a cancelled or failed pick (a dismissed Wayland portal, say) becomes an unhandled
      // rejection. So every path answers through here, and a throw is logged, not propagated.
      const answer = (streams: Electron.Streams) => {
        try {
          callback(streams);
        } catch (error) {
          console.error("display media request could not be answered", error);
        }
      };
      desktopCapturer
        .getSources({
          types: ["screen", "window"],
          thumbnailSize: { width: 320, height: 180 },
          fetchWindowIcons: true,
        })
        .then(async (sources) => {
          const chosen = SYSTEM_PICKER
            ? { id: sources[0]?.id ?? null, systemAudio: false }
            : await askRenderer(request.frame?.processId, sources);
          const source = sources.find((candidate) => candidate.id === chosen.id);
          if (source === undefined) {
            answer({});
            return;
          }
          answer(
            process.platform === "win32" && chosen.systemAudio && source.id.startsWith("screen:")
              ? { video: source, audio: "loopback" }
              : { video: source },
          );
        })
        .catch((error: unknown) => {
          console.error("display media sources could not be listed", error);
          answer({});
        });
    },
    { useSystemPicker: true },
  );
}

/** Shows the renderer the sources and waits for its choice, or for it to dismiss the picker. */
function askRenderer(
  processId: number | undefined,
  sources: Electron.DesktopCapturerSource[],
): Promise<SourceChoice> {
  const target =
    BrowserWindow.getAllWindows().find((w) => w.webContents.getOSProcessId() === processId)
      ?.webContents ?? BrowserWindow.getAllWindows()[0]?.webContents;
  if (target === undefined) {
    return Promise.resolve({ id: null, systemAudio: false });
  }
  const listed = sources.map((source) => ({
    id: source.id,
    name: source.name,
    kind: source.id.startsWith("screen:") ? "screen" : "window",
    thumbnail: source.thumbnail.toDataURL(),
    icon: source.appIcon.isEmpty() ? null : source.appIcon.toDataURL(),
  }));
  return new Promise((resolve) => {
    const onChoice = (event: Electron.IpcMainEvent, choice: SourceChoice) => {
      if (event.sender === target) {
        ipcMain.off("display:source-chosen", onChoice);
        resolve(choice);
      }
    };
    ipcMain.on("display:source-chosen", onChoice);
    target.send("display:pick-source", {
      sources: listed,
      systemAudio: process.platform === "win32",
    });
  });
}

app
  .whenReady()
  .then(() => {
    if (!firstInstance) {
      return;
    }
    serveAppLinks();
    serveChromiumNotices();
    serveDisplayMedia();
    serveGameCapture();
    servePasskeyHandoff();
    serveZoom();
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
