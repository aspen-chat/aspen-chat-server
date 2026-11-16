import { useLayoutEffect, useRef, useSyncExternalStore, type RefObject } from "react";
import {
  AspenCall,
  nativeCalls,
  type PictureShape,
  type TileRect,
  type TileReport,
} from "@/api/nativeCall";

/**
 * Where the mobile apps' native video goes. The phone's call engine draws each picture it
 * receives, and the camera it sends, natively behind the web view (`voice/native-calls.md`,
 * Video): the page leaves a transparent hole where each picture goes and draws names, buttons,
 * popovers, and dialogs over it as it does over a `<video>`. Nothing is ever drawn above the
 * page.
 *
 * Three things cross the bridge for it. The page reports every hole's rectangle
 * (`AspenCall.setTiles`), once when it settles and every animation frame while it moves, since
 * the native picture can only follow the page's layout; the native side reports each picture's
 * own shape back (`shape` events), which sizes a camera's tile as a `<video>`'s dimensions do;
 * and it says when the system shows the call as a picture in picture (`pip` events). All three
 * live here, outside React's render, and components read them through the hooks below.
 *
 * The holes are cut through the page by `styles.css`: while any hole is on the page the root
 * carries `data-video-holes`, under which every ancestor of a hole (`[data-video-hole]`) paints
 * no background, and `NativeVideoBackdrop` (`NativeVideoBackdrop.tsx`) paints the page's ground
 * beneath everything with the holes' rectangles cut out, so what was behind each hole is the
 * native picture and nothing else changes. A hole is a rectangle; a tile's rounded corners are
 * an opaque frame the tile draws over the hole's corners in the ground's colour.
 */

/** A hole's rectangle on the screen, in CSS pixels of the page, as the native side is told it. */
export type HoleRect = TileRect;

/**
 * How one tile draws its picture, beside where: whether it is on screen at all, the self
 * preview mirrored, the picture cropped to fill the hole (`cover`) or whole within it,
 * filling the screen (`fullScreen`: the native side then takes the screen's bounds itself,
 * which it has before the page's layout after a turn), and whether the system's picture in
 * picture shows it (`pictureInPicture`: at most one tile's).
 */
export interface HoleOptions {
  readonly mirrored: boolean;
  readonly cover: boolean;
  readonly fullScreen: boolean;
  readonly pictureInPicture: boolean;
}

interface Hole {
  readonly surface: string;
  readonly element: HTMLElement;
  readonly options: HoleOptions;
}

/**
 * How many animation frames in a row a hole's rectangle must stand still before it is left
 * alone until something moves it again: two, so a transition's last frame is reported.
 */
const SETTLED_FRAMES = 2;

type Listener = () => void;

/**
 * Every hole on the page, by tile, and what was last told of each. One tile, one hole; two
 * tiles may show one surface (a share's thumbnail and the focused screen), each a hole of its
 * own.
 */
class NativeVideoLayout {
  readonly #holes = new Map<string, Hole>();
  /** What `setTiles` was last given, as JSON, so an unchanged layout is not sent again. */
  #told = "";
  /** The rectangles last told, for the backdrop. */
  #rects: readonly HoleRect[] = [];
  readonly #listeners = new Set<Listener>();
  #frame: number | null = null;
  /** How many frames in a row every hole stood still. */
  #still = 0;
  #tiles = 0;

  /** A tile id of this page's own, unique for the page's life. */
  tileId(): string {
    this.#tiles += 1;
    return `tile-${String(this.#tiles)}`;
  }

  /** Puts `tile`'s hole on the page, or moves what it shows, and measures it on the next frame. */
  set(tile: string, hole: Hole): void {
    this.#holes.set(tile, hole);
    this.#move();
  }

  remove(tile: string): void {
    if (this.#holes.delete(tile)) {
      this.#move();
    }
  }

  /** Something moved: measure every frame until every hole stands still. */
  #move(): void {
    this.#still = 0;
    if (this.#frame === null) {
      this.#frame = requestAnimationFrame(this.#tick);
    }
  }

  /** A hint from outside that holes may be moving (a scroll, a transition, a turn). */
  readonly moved = (): void => {
    if (this.#holes.size > 0) {
      this.#move();
    }
  };

  readonly #tick = (): void => {
    this.#frame = null;
    const changed = this.#measure();
    this.#still = changed ? 0 : this.#still + 1;
    if (this.#still < SETTLED_FRAMES) {
      this.#frame = requestAnimationFrame(this.#tick);
    }
  };

  /**
   * Reads every hole's rectangle, in the page's CSS pixels as they appear on screen (the visual
   * viewport's offset and scale applied, for a page pinched), and tells the native side when
   * anything differs from what it was last told; whether it did.
   */
  #measure(): boolean {
    const viewport = window.visualViewport;
    const offsetX = viewport?.offsetLeft ?? 0;
    const offsetY = viewport?.offsetTop ?? 0;
    const scale = viewport?.scale ?? 1;
    const tiles: TileReport[] = [];
    for (const [tile, hole] of this.#holes) {
      const box = hole.element.getBoundingClientRect();
      const rect: HoleRect = {
        x: round((box.left - offsetX) * scale),
        y: round((box.top - offsetY) * scale),
        width: round(box.width * scale),
        height: round(box.height * scale),
      };
      const visible =
        hole.element.isConnected &&
        rect.width > 0 &&
        rect.height > 0 &&
        getComputedStyle(hole.element).visibility !== "hidden";
      tiles.push({ tile, surface: hole.surface, rect, visible, ...hole.options });
    }
    const key = JSON.stringify(tiles);
    if (key === this.#told) {
      return false;
    }
    this.#told = key;
    this.#rects = tiles.filter((tile) => tile.visible).map((tile) => tile.rect);
    document.documentElement.toggleAttribute("data-video-holes", this.#rects.length > 0);
    AspenCall.setTiles({ tiles }).catch((error: unknown) => {
      console.error(error);
    });
    for (const listener of Array.from(this.#listeners)) {
      listener();
    }
    return true;
  }

  /** The rectangles of the holes on screen, as last told. */
  get rects(): readonly HoleRect[] {
    return this.#rects;
  }

  subscribe(listener: Listener): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }
}

/** Half a CSS pixel is as fine as a layout is; it keeps a settled layout from reading as motion. */
function round(value: number): number {
  return Math.round(value * 2) / 2;
}

export const nativeVideoLayout = new NativeVideoLayout();

/** What may move a hole without the hole's own box changing, each heard once for the page. */
let listening = false;
function listenForMotion(): void {
  if (listening || typeof window === "undefined") {
    return;
  }
  listening = true;
  const moved = nativeVideoLayout.moved;
  // A scroll anywhere moves whatever is inside the scroller; a transition or an animation
  // moves what it runs on, and may move the holes around it; a turn or the keyboard changes
  // the viewport, and a pinch the visual viewport.
  for (const name of ["scroll", "transitionrun", "animationstart", "animationiteration"]) {
    document.addEventListener(name, moved, { capture: true, passive: true });
  }
  window.addEventListener("resize", moved);
  window.addEventListener("orientationchange", moved);
  window.visualViewport?.addEventListener("resize", moved);
  window.visualViewport?.addEventListener("scroll", moved);
}

/**
 * Makes the element `hole` refers to a hole for `surface`'s native picture while it is
 * mounted: the picture is drawn behind it, following its rectangle as the page lays it out
 * (its own size through a `ResizeObserver`, and its place every animation frame while
 * anything on the page scrolls, transitions, or animates, until it stands still).
 */
export function useNativeHole(
  surface: string,
  hole: RefObject<HTMLElement | null>,
  options: HoleOptions,
): void {
  const tile = useRef<string | null>(null);
  const { mirrored, cover, fullScreen, pictureInPicture } = options;
  useLayoutEffect(() => {
    const element = hole.current;
    if (element === null) {
      return;
    }
    listenForMotion();
    tile.current ??= nativeVideoLayout.tileId();
    const id = tile.current;
    nativeVideoLayout.set(id, {
      surface,
      element,
      options: { mirrored, cover, fullScreen, pictureInPicture },
    });
    const observer = new ResizeObserver(nativeVideoLayout.moved);
    observer.observe(element);
    return () => {
      observer.disconnect();
      nativeVideoLayout.remove(id);
    };
  }, [surface, hole, mirrored, cover, fullScreen, pictureInPicture]);
}

// --- Shapes -------------------------------------------------------------------------------------

/** The shape of each surface's picture, as the native side last reported it. */
class NativeVideoShapes {
  readonly #shapes = new Map<string, PictureShape>();
  readonly #listeners = new Set<Listener>();
  #listening = false;

  /** Starts hearing the plugin's `shape` events, once. */
  listen(): void {
    if (this.#listening || !nativeCalls()) {
      return;
    }
    this.#listening = true;
    void AspenCall.addListener("shape", (shape) => {
      const current = this.#shapes.get(shape.surface);
      if (
        current?.width === shape.width &&
        current.height === shape.height &&
        current.rotation === shape.rotation
      ) {
        return;
      }
      this.#shapes.set(shape.surface, shape);
      for (const listener of Array.from(this.#listeners)) {
        listener();
      }
    });
  }

  get(surface: string): PictureShape | null {
    return this.#shapes.get(surface) ?? null;
  }

  subscribe(listener: Listener): () => void {
    this.listen();
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }
}

export const nativeVideoShapes = new NativeVideoShapes();

/**
 * The size of the picture drawn for `surface`, upright (a phone's rotation applied), as its
 * first frame arrives and whenever it changes; `null` before the first, and for no surface (a
 * tile drawing a track).
 */
export function useNativePictureShape(
  surface: string | null,
): { width: number; height: number } | null {
  const shape = useSyncExternalStore(
    (listener) => (surface === null ? () => undefined : nativeVideoShapes.subscribe(listener)),
    () => (surface === null ? null : nativeVideoShapes.get(surface)),
    () => null,
  );
  return shape === null ? null : { width: shape.width, height: shape.height };
}

// --- Picture in picture ---------------------------------------------------------------------------

/** Whether the system shows the call as a picture in picture now, as the plugin's `pip` events say. */
class NativePictureInPicture {
  #active = false;
  readonly #listeners = new Set<Listener>();
  #listening = false;

  listen(): void {
    if (this.#listening || !nativeCalls()) {
      return;
    }
    this.#listening = true;
    void AspenCall.addListener("pip", ({ active }) => {
      if (active !== this.#active) {
        this.#active = active;
        for (const listener of Array.from(this.#listeners)) {
          listener();
        }
      }
    });
  }

  get active(): boolean {
    return this.#active;
  }

  subscribe(listener: Listener): () => void {
    this.listen();
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }
}

export const nativePictureInPicture = new NativePictureInPicture();

/**
 * Whether the system shows the call as a picture in picture: on Android the whole page is
 * shown small then, so the call screen shows the one picture alone; on iOS the system draws
 * the picture itself and the page is unaffected.
 */
export function usePictureInPicture(): boolean {
  return useSyncExternalStore(
    (listener) => nativePictureInPicture.subscribe(listener),
    () => nativePictureInPicture.active,
    () => false,
  );
}
