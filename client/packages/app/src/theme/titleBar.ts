import { useSyncExternalStore } from "react";

/**
 * Whether the page is laid out under a window's buttons, as the desktop app's is when it draws
 * its own title bar (`packages/desktop/src/main/titleBar.ts`), through Chromium's Window Controls
 * Overlay. While it is, `data-title-bar` is on <html>, and `styles.css` keeps everything clear of
 * the strip (`--window-top`), which `TitleBar.tsx` draws.
 */

const overlay = typeof navigator === "undefined" ? undefined : navigator.windowControlsOverlay;

const listeners = new Set<() => void>();

function shown(): boolean {
  return overlay?.visible ?? false;
}

function show(): void {
  if (shown()) {
    document.documentElement.dataset.titleBar = "";
  } else {
    delete document.documentElement.dataset.titleBar;
  }
  for (const listener of listeners) {
    listener();
  }
}

/** Marks the root while the page draws the title bar, and follows it. Called once, as the page starts. */
export function followTitleBar(): void {
  if (overlay === undefined) {
    return;
  }
  show();
  overlay.addEventListener("geometrychange", show);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Whether the page draws the window's title bar. */
export function useTitleBarShown(): boolean {
  return useSyncExternalStore(subscribe, shown);
}

/**
 * A computed colour as `#rrggbb`, the one form the desktop shell takes, or `null` if it cannot be
 * read. Drawn on a canvas, since computed colours come in whatever space they were mixed in
 * (`oklab(…)` for the rail's) and a canvas reads any of them back as sRGB.
 */
export function hexColor(css: string): string | null {
  const context = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  if (context === null) {
    return null;
  }
  context.fillStyle = css;
  context.fillRect(0, 0, 1, 1);
  const [red, green, blue] = context.getImageData(0, 0, 1, 1).data;
  if (red === undefined || green === undefined || blue === undefined) {
    return null;
  }
  return `#${[red, green, blue].map((part) => part.toString(16).padStart(2, "0")).join("")}`;
}
