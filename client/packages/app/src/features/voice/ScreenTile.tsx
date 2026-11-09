import { CornersInIcon, CornersOutIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { isFullScreenKey, useFullScreen, useOrientationLock } from "@/features/voice/fullScreen";
import { useMessages } from "@/i18n/context";

/**
 * One shared screen or camera, playing. The audio that came with it is played by the call
 * itself, so the element is muted and only shows the picture. A local preview is the sender's
 * own track, which a camera shows `mirrored`, as a mirror would. An `expandable` tile can be
 * made full screen with its button or a double click, and with the F key where `shortcut` says
 * it is the one tile on screen the key means; full screen also turns a phone to the picture's
 * orientation. `caption` names the picture over its corner, which a tile already named beside
 * it leaves out. A `natural` tile takes the picture's own shape, whatever it is, as its frames
 * arrive (16:9 until the first does), no taller than `NATURAL_MAX_HEIGHT`, narrowing to keep the
 * shape where that would be exceeded; its `className` then only decorates it.
 */
/** The tallest a `natural` tile grows, so a portrait camera on a phone leaves room around it. */
const NATURAL_MAX_HEIGHT = "70vh";

export function ScreenTile({
  track,
  label,
  className,
  expandable = false,
  shortcut = expandable,
  mirrored = false,
  caption = true,
  natural = false,
}: {
  track: MediaStreamTrack;
  label: string;
  className?: string;
  expandable?: boolean;
  shortcut?: boolean;
  mirrored?: boolean;
  caption?: boolean;
  natural?: boolean;
}) {
  const m = useMessages();
  const figure = useRef<HTMLElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const full = useFullScreen(figure);
  const { toggle } = full;
  useOrientationLock(full.state, video);
  useEffect(() => {
    if (!shortcut) {
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
  }, [shortcut, toggle]);
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
  const shape = usePictureShape(natural ? video : null);
  const sized: CSSProperties | undefined =
    natural && shape !== null
      ? {
          aspectRatio: `${String(shape.width)} / ${String(shape.height)}`,
          width: `min(100%, calc(${NATURAL_MAX_HEIGHT} * ${String(shape.width / shape.height)}))`,
        }
      : undefined;
  const filling = full.state === "window";
  return (
    <figure
      ref={figure}
      onDoubleClick={expandable ? full.toggle : undefined}
      className={
        "group overflow-hidden bg-black [&:fullscreen]:rounded-none " +
        (filling
          ? "fixed inset-x-0 top-(--title-bar-height) bottom-0 z-50 p-[env(safe-area-inset-top)_env(safe-area-inset-right)_env(safe-area-inset-bottom)_env(safe-area-inset-left)]"
          : "relative rounded-lg " +
            (natural ? (sized === undefined ? "aspect-video w-full " : "mx-auto ") : "") +
            (className ?? ""))
      }
      style={filling ? undefined : sized}
    >
      <video
        ref={video}
        autoPlay
        playsInline
        muted
        aria-label={label}
        className={"h-full w-full object-contain" + (mirrored ? " -scale-x-100" : "")}
      />
      {caption && (
        <figcaption className="absolute bottom-2 start-2 rounded-md bg-black/60 px-2 py-0.5 text-xs text-white">
          {label}
        </figcaption>
      )}
      {expandable && (
        <button
          type="button"
          onClick={full.toggle}
          aria-label={full.state === "off" ? m.voice.fullScreen : m.voice.exitFullScreen}
          aria-keyshortcuts={shortcut ? "F" : undefined}
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

/**
 * The size of the picture `video` plays, as its first frame arrives and whenever it changes (a
 * camera's resolution climbing, a phone turning); `null` before the first, or with no element.
 */
function usePictureShape(
  video: RefObject<HTMLVideoElement | null> | null,
): { width: number; height: number } | null {
  const [shape, setShape] = useState<{ width: number; height: number } | null>(null);
  useEffect(() => {
    const element = video?.current;
    if (element === null || element === undefined) {
      return;
    }
    const read = () => {
      const { videoWidth: width, videoHeight: height } = element;
      if (width > 0 && height > 0) {
        setShape((current) =>
          current?.width === width && current.height === height ? current : { width, height },
        );
      }
    };
    element.addEventListener("loadedmetadata", read);
    element.addEventListener("resize", read);
    return () => {
      element.removeEventListener("loadedmetadata", read);
      element.removeEventListener("resize", read);
    };
  }, [video]);
  return shape;
}
