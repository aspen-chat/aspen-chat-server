import { CornersInIcon, CornersOutIcon } from "@phosphor-icons/react";
import { useEffect, useRef } from "react";
import { isFullScreenKey, useFullScreen } from "@/features/voice/fullScreen";
import { useMessages } from "@/i18n/context";

/**
 * One shared screen, playing. The audio that came with it is played by the call itself, so the
 * element is muted and only shows the picture. A local preview is the sender's own track.
 * An `expandable` tile can be made full screen with its button, a double click, or the F key.
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
  const { toggle } = full;
  useEffect(() => {
    if (!expandable) {
      return;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (isFullScreenKey(event)) {
        event.preventDefault();
        toggle();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [expandable, toggle]);
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
          aria-keyshortcuts="F"
          title={full.state === "off" ? m.voice.fullScreenHint : m.voice.exitFullScreenHint}
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
