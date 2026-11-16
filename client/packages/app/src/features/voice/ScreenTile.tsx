import type { VideoSource } from "@aspen/protocol";
import { CornersInIcon, CornersOutIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { isFullScreenKey, useFullScreen, useOrientationLock } from "@/features/voice/fullScreen";
import { useNativeHole, useNativePictureShape } from "@/features/voice/nativeVideo";
import { useMessages } from "@/i18n/context";

/**
 * One shared screen or camera, playing. The audio that came with it is played by the call
 * itself, so the element is muted and only shows the picture. A local preview is the sender's
 * own picture, which a camera shows `mirrored`, as a mirror would. An `expandable` tile can be
 * made full screen with its button or a double click, and with the F key where `shortcut` says
 * it is the one tile on screen the key means; full screen also turns a phone to the picture's
 * orientation. `caption` names the picture over its corner, which a tile already named beside
 * it leaves out. A `natural` tile takes the picture's own shape, whatever it is, as its frames
 * arrive (16:9 until the first does), no taller than `NATURAL_MAX_HEIGHT`, narrowing to keep the
 * shape where that would be exceeded; its `className` then only decorates it. A `cover` tile
 * fills its box with the picture, cropped, where the box's shape is the layout's (a phone's grid
 * of cells); otherwise the whole picture shows within the box. A tile that is not `rounded`
 * (the one picture a phone's picture in picture shows) reaches its box's edges.
 *
 * The picture is one of two kinds (`VideoSource`). A track gets a `<video>` of its own: WebKit
 * on iOS, handed a phone's other camera on the element that played the first, keeps laying the
 * picture out as the first camera's rotated frames were and turns it upright afterwards, so a
 * portrait picture is drawn at the size of a landscape one fitted to the tile, in a black
 * square. A native surface, which the phone's call engine draws behind the page, gets a hole:
 * the figure paints nothing (`data-video-hole`, under which `styles.css` leaves its ancestors
 * unpainted too), reports its rectangle to the native side (`useNativeHole`), and draws its
 * rounded corners as an opaque frame over the hole in the page's ground colour, since the native
 * picture is a rectangle. Its shape comes back from the native side (`useNativePictureShape`).
 * Full screen is told to the native side, which then takes the screen's bounds itself.
 */
/** The tallest a `natural` tile grows, so a portrait camera on a phone leaves room around it. */
const NATURAL_MAX_HEIGHT = "70vh";

/** The size of a picture, in its own pixels, upright. */
export interface PictureSize {
  readonly width: number;
  readonly height: number;
}

export function ScreenTile({
  source,
  label,
  className,
  expandable = false,
  shortcut = expandable,
  mirrored = false,
  caption = true,
  natural = false,
  cover = false,
  rounded = true,
  pictureInPicture = false,
}: {
  source: VideoSource;
  label: string;
  className?: string;
  expandable?: boolean;
  shortcut?: boolean;
  mirrored?: boolean;
  caption?: boolean;
  natural?: boolean;
  cover?: boolean;
  rounded?: boolean;
  /** Whether the system's picture in picture shows this picture (one tile of the call's). */
  pictureInPicture?: boolean;
}) {
  const m = useMessages();
  const figure = useRef<HTMLElement>(null);
  const full = useFullScreen(figure);
  const { toggle } = full;
  const [trackShape, setTrackShape] = useState<PictureSize | null>(null);
  const shape = usePictureShape(source, trackShape);
  useOrientationLock(full.state, shape);
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
  const sized: CSSProperties | undefined =
    natural && shape !== null
      ? {
          aspectRatio: `${String(shape.width)} / ${String(shape.height)}`,
          width: `min(100%, calc(${NATURAL_MAX_HEIGHT} * ${String(shape.width / shape.height)}))`,
        }
      : undefined;
  const filling = full.state === "window";
  const native = source.kind === "native";
  return (
    <figure
      ref={figure}
      onDoubleClick={expandable ? full.toggle : undefined}
      className={
        "group [&:fullscreen]:rounded-none " +
        // A hole's corners are painted by its frame, which the figure must not clip to its
        // rounded shape; a `<video>` is clipped to it.
        (native ? "" : "overflow-hidden bg-black ") +
        (filling
          ? "fixed inset-x-0 top-(--title-bar-height) bottom-0 z-50 p-[env(safe-area-inset-top)_env(safe-area-inset-right)_env(safe-area-inset-bottom)_env(safe-area-inset-left)]"
          : "relative " +
            (rounded ? "rounded-lg " : "") +
            (natural ? (sized === undefined ? "aspect-video w-full " : "mx-auto ") : "") +
            (className ?? ""))
      }
      style={filling ? undefined : sized}
    >
      <Picture
        source={source}
        label={label}
        mirrored={mirrored}
        cover={cover}
        framed={rounded && !filling}
        fullScreen={filling}
        pictureInPicture={pictureInPicture}
        onShape={setTrackShape}
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
 * The picture itself, by its source's kind: a `<video>` for a track, which reports the size of
 * what it plays through `onShape`, or a hole for a surface.
 */
function Picture({
  source,
  label,
  mirrored,
  cover,
  framed,
  fullScreen,
  pictureInPicture,
  onShape,
}: {
  source: VideoSource;
  label: string;
  mirrored: boolean;
  cover: boolean;
  framed: boolean;
  fullScreen: boolean;
  pictureInPicture: boolean;
  onShape: (shape: PictureSize) => void;
}) {
  switch (source.kind) {
    case "track":
      return (
        <TrackPicture
          track={source.track}
          label={label}
          mirrored={mirrored}
          cover={cover}
          onShape={onShape}
        />
      );
    case "native":
      return (
        <NativePicture
          surface={source.surface}
          label={label}
          mirrored={mirrored}
          cover={cover}
          framed={framed}
          fullScreen={fullScreen}
          pictureInPicture={pictureInPicture}
        />
      );
    default:
      return source satisfies never;
  }
}

/**
 * A `<video>` playing `track`, made anew for each track (`key`), telling `onShape` the size of
 * the picture as its first frame arrives and whenever it changes (a camera's resolution
 * climbing, a phone turning).
 */
function TrackPicture({
  track,
  label,
  mirrored,
  cover,
  onShape,
}: {
  track: MediaStreamTrack;
  label: string;
  mirrored: boolean;
  cover: boolean;
  onShape: (shape: PictureSize) => void;
}) {
  const video = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const element = video.current;
    if (element === null) {
      return;
    }
    element.srcObject = new MediaStream([track]);
    const read = () => {
      if (element.videoWidth > 0 && element.videoHeight > 0) {
        onShape({ width: element.videoWidth, height: element.videoHeight });
      }
    };
    read();
    element.addEventListener("loadedmetadata", read);
    element.addEventListener("resize", read);
    return () => {
      element.removeEventListener("loadedmetadata", read);
      element.removeEventListener("resize", read);
      element.srcObject = null;
    };
  }, [track, onShape]);
  return (
    <video
      key={track.id}
      ref={video}
      autoPlay
      playsInline
      muted
      aria-label={label}
      className={
        "h-full w-full " +
        (cover ? "object-cover" : "object-contain") +
        (mirrored ? " -scale-x-100" : "")
      }
    />
  );
}

/**
 * A hole for the picture the phone draws behind the page, named for assistive technology as the
 * `<video>` would be. Where it is `framed`, the tile's rounded corners are drawn over its corners
 * as an opaque frame in the page's ground colour: a ring of the ground as wide as any screen,
 * clipped by a wrapper to the figure's box, so only the corners outside the rounded rectangle are
 * painted.
 */
function NativePicture({
  surface,
  label,
  mirrored,
  cover,
  framed,
  fullScreen,
  pictureInPicture,
}: {
  surface: string;
  label: string;
  mirrored: boolean;
  cover: boolean;
  framed: boolean;
  fullScreen: boolean;
  pictureInPicture: boolean;
}) {
  const hole = useRef<HTMLDivElement>(null);
  useNativeHole(surface, hole, {
    mirrored,
    cover,
    fullScreen,
    pictureInPicture,
  });
  return (
    <div
      ref={hole}
      role="img"
      aria-label={label}
      data-video-hole={surface}
      className="h-full w-full"
    >
      {framed && (
        <div aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-hidden">
          <div className="absolute inset-0 rounded-lg shadow-[0_0_0_200vmax_var(--color-surface)]" />
        </div>
      )}
    </div>
  );
}

/**
 * The size of the picture a tile draws; `null` before the first frame. A track's is what its
 * `<video>` last reported (`trackShape`), kept across a change of track until the new one's
 * arrives; a native surface's is what the phone reports of the picture it draws.
 */
function usePictureShape(source: VideoSource, trackShape: PictureSize | null): PictureSize | null {
  const nativeShape = useNativePictureShape(source.kind === "native" ? source.surface : null);
  switch (source.kind) {
    case "track":
      return trackShape;
    case "native":
      return nativeShape;
    default:
      return source satisfies never;
  }
}
