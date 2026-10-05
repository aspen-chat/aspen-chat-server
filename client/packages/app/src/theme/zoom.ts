import { useSyncExternalStore } from "react";

/**
 * How large the whole app is drawn in the desktop app, which zooms its page as a browser does
 * (`packages/desktop/src/main/zoom.ts`, which also takes Ctrl + and Ctrl − and asks the page to
 * step) and keeps the factor, applying it before the page loads so nothing jumps at launch.
 * Elsewhere the app has no zoom of its own: a browser's zoom is the one, and a page can neither
 * read nor set it, and the phones' own display and text size settings are followed instead
 * (`theme/systemTextSize.ts`).
 */

/** The least and the most the app may be zoomed. */
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 3;

/** The factors Ctrl + and Ctrl − step between, as browsers have them. */
export const ZOOM_STEPS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3] as const;

/** How near a factor must be to a step to count as on it, for factors set by the slider. */
const ON_STEP = 0.001;

/** A step of the zoom: in, out, or back to normal. */
export type ZoomStep = 1 | -1 | 0;

/**
 * The factor one step from `factor`: the next of `ZOOM_STEPS` above or below it, so a factor
 * between two steps goes to the nearer one in that direction, or 1 to go back to normal.
 */
export function steppedZoom(factor: number, step: ZoomStep): number {
  if (step === 0) {
    return 1;
  }
  if (step > 0) {
    return ZOOM_STEPS.find((level) => level > factor + ON_STEP) ?? MAX_ZOOM;
  }
  return ZOOM_STEPS.findLast((level) => level < factor - ON_STEP) ?? MIN_ZOOM;
}

/** A zoom factor within the bounds, or `undefined` for anything else. */
export function zoomFactor(raw: unknown): number | undefined {
  return typeof raw === "number" && Number.isFinite(raw)
    ? Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, raw))
    : undefined;
}

const host = typeof window === "undefined" ? null : (window.aspenDesktop?.zoom ?? null);

/** Whether this shell zooms the page, so the app offers its own zoom. */
export const zoomAvailable = host !== null;

/** The factor the shell last reported or was given; `null` until it is first read. */
let current: number | null = null;
const listeners = new Set<() => void>();

function changeTo(factor: number): void {
  current = factor;
  for (const listener of listeners) {
    listener();
  }
}

/** Zooms the app to `factor`, kept by the shell for the next launch. */
export function setZoom(factor: number): void {
  const bounded = zoomFactor(factor);
  if (host === null || bounded === undefined) {
    return;
  }
  changeTo(bounded);
  host.set(bounded).catch((error: unknown) => {
    console.error("the zoom could not be set", error);
  });
}

/**
 * Reads the shell's zoom, and steps it as Ctrl + and Ctrl − ask. Called once, as the page
 * starts.
 */
export function followZoom(): void {
  if (host === null) {
    return;
  }
  host
    .get()
    .then((factor) => {
      changeTo(zoomFactor(factor) ?? 1);
    })
    .catch((error: unknown) => {
      console.error("the zoom could not be read", error);
    });
  host.onStep((step) => {
    setZoom(steppedZoom(current ?? 1, step));
  });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The app's zoom factor, or `null` while it is unknown or where the shell does not zoom. */
export function useZoom(): number | null {
  return useSyncExternalStore(subscribe, () => current);
}
