import { Capacitor, type PluginListenerHandle } from "@capacitor/core";
import { useCallback, useEffect, useState, type RefObject } from "react";

/**
 * Whether a key press toggles full screen: F with no modifier, by the letter it types or, in a
 * layout without Latin letters, by the key in F's place, and never while the user types or
 * works in a dialog.
 */
export function isFullScreenKey(event: KeyboardEvent): boolean {
  if (event.repeat || event.ctrlKey || event.metaKey || event.altKey) {
    return false;
  }
  const latin = /^[a-z]$/i.test(event.key);
  if (!(latin ? event.key.toLowerCase() === "f" : event.code === "KeyF")) {
    return false;
  }
  const target = event.target;
  return !(
    target instanceof Element &&
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role=dialog], [role=menu], [role=listbox]',
    ) !== null
  );
}

/**
 * Whether an element is full screen: `screen` through the Fullscreen API, which the browser
 * leaves on Escape, or `window`, filling the app's window, left with Escape or the button.
 * The window is the fallback where the API is missing or refuses, and the only way in the
 * mobile apps, whose Capacitor WebView dismisses any element that asks for the screen; there
 * the system's back gesture leaves it too, and does nothing else meanwhile.
 */
export type FullScreenState = "off" | "screen" | "window";

export function useFullScreen(element: RefObject<HTMLElement | null>) {
  const [state, setState] = useState<FullScreenState>("off");

  useEffect(() => {
    const changed = () => {
      setState((current) =>
        document.fullscreenElement !== null && document.fullscreenElement === element.current
          ? "screen"
          : current === "screen"
            ? "off"
            : current,
      );
    };
    document.addEventListener("fullscreenchange", changed);
    return () => {
      document.removeEventListener("fullscreenchange", changed);
    };
  }, [element]);

  useEffect(() => {
    if (state !== "window") {
      return;
    }
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setState("off");
      }
    };
    window.addEventListener("keydown", escape);
    // While any listener is registered Capacitor leaves back to it alone, so it is registered
    // only for as long as the window is filled.
    let cancelled = false;
    let back: PluginListenerHandle | null = null;
    if (Capacitor.isNativePlatform()) {
      void import("@capacitor/app")
        .then(({ App }) =>
          App.addListener("backButton", () => {
            setState("off");
          }),
        )
        .then((handle) => {
          if (cancelled) {
            void handle.remove();
          } else {
            back = handle;
          }
        });
    }
    return () => {
      cancelled = true;
      window.removeEventListener("keydown", escape);
      void back?.remove();
    };
  }, [state]);

  const toggle = useCallback(() => {
    const target = element.current;
    if (state === "screen") {
      void document.exitFullscreen();
    } else if (state === "window") {
      setState("off");
    } else if (target !== null) {
      if (!Capacitor.isNativePlatform() && document.fullscreenEnabled) {
        target.requestFullscreen().catch(() => {
          setState("window");
        });
      } else {
        setState("window");
      }
    }
  }, [element, state]);

  return { state, toggle };
}

/** The orientation that shows a picture of this size largest, or none before it has one. */
export function orientationFor(width: number, height: number): "landscape" | "portrait" | null {
  if (width <= 0 || height <= 0) {
    return null;
  }
  return width >= height ? "landscape" : "portrait";
}

/**
 * Turns the screen to the picture's orientation while it is full screen, and frees it again
 * after. Where the orientation cannot be locked (a desktop, a browser outside full screen), the
 * request is refused and nothing changes.
 */
export function useOrientationLock(
  state: FullScreenState,
  video: RefObject<HTMLVideoElement | null>,
) {
  useEffect(() => {
    const element = video.current;
    const orientation =
      state === "off" || element === null
        ? null
        : orientationFor(element.videoWidth, element.videoHeight);
    if (orientation === null) {
      return;
    }
    const plugin = import("@capacitor/screen-orientation").then(
      ({ ScreenOrientation }) => ScreenOrientation,
    );
    void plugin.then((screen) => screen.lock({ orientation })).catch(() => undefined);
    return () => {
      void plugin.then((screen) => screen.unlock()).catch(() => undefined);
    };
  }, [state, video]);
}
