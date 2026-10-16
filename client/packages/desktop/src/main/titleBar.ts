import { BrowserWindow, app, ipcMain } from "electron";
import { readFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * The window's title bar, drawn by the page in the community rail's colour: the system's own
 * title bar is hidden, and only its window buttons are drawn, over the page's top edge (the
 * Window Controls Overlay), in the colours the page last sent (`title-bar:paint`). The page lays
 * itself out below the overlay and draws the strip beside the buttons itself
 * (`packages/app/src/features/layout/TitleBar.tsx`).
 *
 * On Linux the reader may keep the system's title bar instead, since a desktop's own window
 * decorations (KDE's) or a tiling window manager's lack of any are often what they want there; a
 * window's frame is fixed when it is made, so the choice applies from the next launch. What is
 * kept is `title-bar.json` in the profile: that choice, and the colours, so a window opens in
 * the ones it closed in.
 */

/**
 * How tall the page's strip is, in its own pixels, at a zoom of 1: the 2rem floor of
 * `--title-bar-height` in `packages/app/src/styles.css`.
 */
const STRIP_HEIGHT = 32;

/**
 * The least height the window buttons are drawn at, in the screen's pixels, however far the page
 * is zoomed out.
 */
const MIN_HEIGHT = 28;

/** Whether this platform lets the reader keep the system's title bar instead. */
const CHOOSABLE = process.platform === "linux";

interface Kept {
  drawn: boolean;
  color: string | null;
  symbolColor: string | null;
}

function keptFile(): string {
  return join(app.getPath("userData"), "title-bar.json");
}

/** A colour as `#rrggbb`, the one form the page sends, or `null` for anything else. */
function hexColor(raw: unknown): string | null {
  return typeof raw === "string" && /^#[0-9a-f]{6}$/i.test(raw) ? raw : null;
}

function readKept(): Kept {
  try {
    const raw: unknown = JSON.parse(readFileSync(keptFile(), "utf8"));
    if (typeof raw === "object" && raw !== null) {
      return {
        drawn: !("drawn" in raw) || raw.drawn !== false,
        color: "color" in raw ? hexColor(raw.color) : null,
        symbolColor: "symbolColor" in raw ? hexColor(raw.symbolColor) : null,
      };
    }
  } catch {
    // None kept yet, or unreadable: drawn, in the system's colours until the page sends its own.
  }
  return { drawn: true, color: null, symbolColor: null };
}

let kept: Kept | null = null;

function current(): Kept {
  kept ??= readKept();
  return kept;
}

let keeping = Promise.resolve();

/** Writes what is kept, one write at a time and in order, so the last change is what stays. */
function keep(next: Kept): Promise<void> {
  kept = next;
  keeping = keeping.then(async () => {
    try {
      await writeFile(keptFile(), JSON.stringify(next));
    } catch (error) {
      console.error("the title bar's settings could not be kept", error);
    }
  });
  return keeping;
}

let drawnThisLaunch: boolean | null = null;

/**
 * Whether windows made in this launch draw their own title bar: read once, after the profile's
 * place is settled, and held for the launch, since a window's frame cannot change.
 */
function drawn(): boolean {
  drawnThisLaunch ??= !CHOOSABLE || current().drawn;
  return drawnThisLaunch;
}

/** The height of the window buttons for a page zoomed by `zoom`, so the strip grows with the page. */
function overlayHeight(zoom: number): number {
  return Math.max(MIN_HEIGHT, Math.round(STRIP_HEIGHT * zoom));
}

/** The title bar options for a window made at `zoom`. */
export function titleBarOptions(zoom: number): Electron.BrowserWindowConstructorOptions {
  if (!drawn()) {
    return {};
  }
  const { color, symbolColor } = current();
  return {
    titleBarStyle: "hidden",
    titleBarOverlay: {
      height: overlayHeight(zoom),
      ...(color === null ? {} : { color }),
      ...(symbolColor === null ? {} : { symbolColor }),
    },
  };
}

/**
 * Fits the window buttons to a page zoomed by `zoom`. macOS sizes its window buttons once, as the
 * window is made, so there the page's strip grows past them instead.
 */
export function fitTitleBar(window: BrowserWindow, zoom: number): void {
  if (drawn() && process.platform !== "darwin") {
    window.setTitleBarOverlay({ height: overlayHeight(zoom) });
  }
}

export function serveTitleBar(): void {
  ipcMain.on("title-bar:paint", (event, raw: unknown) => {
    if (!drawn() || process.platform === "darwin" || typeof raw !== "object" || raw === null) {
      return;
    }
    const color = "color" in raw ? hexColor(raw.color) : null;
    const symbolColor = "symbolColor" in raw ? hexColor(raw.symbolColor) : null;
    if (color === null || symbolColor === null) {
      return;
    }
    BrowserWindow.fromWebContents(event.sender)?.setTitleBarOverlay({ color, symbolColor });
    const was = current();
    if (was.color !== color || was.symbolColor !== symbolColor) {
      void keep({ ...was, color, symbolColor });
    }
  });
  ipcMain.handle("title-bar:choice", () =>
    CHOOSABLE ? { drawn: drawn(), next: current().drawn } : null,
  );
  ipcMain.handle("title-bar:choose", async (_event, raw: unknown) => {
    if (CHOOSABLE && typeof raw === "boolean") {
      await keep({ ...current(), drawn: raw });
    }
  });
  ipcMain.on("title-bar:relaunch", () => {
    app.relaunch();
    app.quit();
  });
}
