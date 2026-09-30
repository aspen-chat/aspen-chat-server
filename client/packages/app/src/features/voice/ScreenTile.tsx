import { Capacitor } from "@capacitor/core";
import { CornersInIcon, CornersOutIcon } from "@phosphor-icons/react";
import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { useMessages } from "@/i18n/context";

/**
 * One shared screen, playing. The audio that came with it is played by the call itself, so the
 * element is muted and only shows the picture. A local preview is the sender's own track.
 * An `expandable` tile can be made full screen with its button or a double click.
 */
export function ScreenTile({
  track,
  label,
  className,
  expandable = false,
}: {
  track: MediaStreamTrack;
  label: string;
  className?: string;
  expandable?: boolean;
}) {
  const m = useMessages();
  const figure = useRef<HTMLElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const full = useFullScreen(figure);
  useEffect(() => {
    const element = video.current;
    if (element === null) {
      return;
    }
    element.srcObject = new MediaStream([track]);
    return () => {
      element.srcObject = null;
    };
  }, [track]);
  const filling = full.state === "window";
  return (
    <figure
      ref={figure}
      onDoubleClick={expandable ? full.toggle : undefined}
      className={
        "group overflow-hidden bg-black [&:fullscreen]:rounded-none " +
        (filling
          ? "fixed inset-0 z-50 p-[env(safe-area-inset-top)_env(safe-area-inset-right)_env(safe-area-inset-bottom)_env(safe-area-inset-left)]"
          : "relative rounded-lg " + (className ?? ""))
      }
    >
      <video
        ref={video}
        autoPlay
        playsInline
        muted
        aria-label={label}
        className="h-full w-full object-contain"
      />
      <figcaption className="absolute bottom-2 start-2 rounded-md bg-black/60 px-2 py-0.5 text-xs text-white">
        {label}
      </figcaption>
      {expandable && (
        <button
          type="button"
          onClick={full.toggle}
          aria-label={full.state === "off" ? m.voice.fullScreen : m.voice.exitFullScreen}
          title={full.state === "off" ? m.voice.fullScreen : m.voice.exitFullScreen}
          className="absolute end-2 top-2 rounded-md bg-black/60 p-1.5 text-white opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent [@media(hover:none)]:opacity-100"
        >
          {full.state === "off" ? (
            <CornersOutIcon size={18} aria-hidden="true" />
          ) : (
            <CornersInIcon size={18} aria-hidden="true" />
          )}
        </button>
      )}
    </figure>
  );
}

/**
 * Whether an element is full screen: `screen` through the Fullscreen API, which the browser
 * leaves on Escape, or `window`, filling the app's window, left with Escape or the button.
 * The window is the fallback where the API is missing or refuses, and the only way in the
 * mobile apps, whose Capacitor WebView dismisses any element that asks for the screen.
 */
type FullScreenState = "off" | "screen" | "window";

function useFullScreen(element: RefObject<HTMLElement | null>) {
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
    return () => {
      window.removeEventListener("keydown", escape);
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
