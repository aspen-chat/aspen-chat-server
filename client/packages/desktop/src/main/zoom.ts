import { BrowserWindow, app, ipcMain } from "electron";
import { readFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * The window's zoom, as a browser's: the factor is kept in the profile (`zoom.json`) and given
 * to each window as it is made, so the page loads at its size. Ctrl + and Ctrl − (⌘ on macOS),
 * Ctrl 0, and Ctrl with the mouse wheel are taken here, before the page or the menu sees them,
 * and handed to the page as steps (`zoom:step`); the page decides the factor
 * (`packages/app/src/theme/zoom.ts`, whose steps its slider in Settings shares) and sets it
 * back (`zoom:set`).
 */

/** The bounds the page keeps the factor within, held here too against a page that does not. */
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 3;

function zoomFile(): string {
  return join(app.getPath("userData"), "zoom.json");
}

function bounded(raw: unknown): number | null {
  return typeof raw === "number" && Number.isFinite(raw)
    ? Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, raw))
    : null;
}

function readZoom(): number {
  try {
    const kept: unknown = JSON.parse(readFileSync(zoomFile(), "utf8"));
    return typeof kept === "object" && kept !== null && "factor" in kept
      ? (bounded(kept.factor) ?? 1)
      : 1;
  } catch {
    // None kept yet, or unreadable: the normal size.
    return 1;
  }
}

let current: number | null = null;

/** The factor each window is made with. */
export function storedZoom(): number {
  current ??= readZoom();
  return current;
}

/** The step a key press asks for, or `null` for any other press. */
function stepOf(input: Electron.Input): 1 | -1 | 0 | null {
  const command = process.platform === "darwin" ? input.meta : input.control;
  if (input.type !== "keyDown" || !command || input.alt) {
    return null;
  }
  switch (input.key) {
    case "+":
    case "=":
      return 1;
    case "-":
    case "_":
      return -1;
    case "0":
      return 0;
    default:
      return null;
  }
}

/** Takes the zoom keys and the wheel from `window`'s page, and applies the kept factor on load. */
export function zoomWindow(window: BrowserWindow): void {
  const contents = window.webContents;
  contents.on("before-input-event", (event, input) => {
    const step = stepOf(input);
    if (step !== null) {
      event.preventDefault();
      contents.send("zoom:step", step);
    }
  });
  contents.on("zoom-changed", (_event, direction) => {
    contents.send("zoom:step", direction === "in" ? 1 : -1);
  });
  // A reload keeps the zoom too.
  contents.on("did-finish-load", () => {
    contents.setZoomFactor(storedZoom());
  });
}

let keeping = Promise.resolve();

export function serveZoom(): void {
  ipcMain.handle("zoom:get", () => storedZoom());
  ipcMain.handle("zoom:set", async (_event, raw: unknown) => {
    const factor = bounded(raw);
    if (factor === null) {
      return;
    }
    current = factor;
    for (const window of BrowserWindow.getAllWindows()) {
      window.webContents.setZoomFactor(factor);
    }
    // One write at a time, in order, so held keys stepping quickly leave the last factor kept.
    keeping = keeping.then(async () => {
      try {
        await writeFile(zoomFile(), JSON.stringify({ factor }));
      } catch (error) {
        console.error("the zoom could not be kept", error);
      }
    });
    await keeping;
  });
}
