import { Capacitor, registerPlugin } from "@capacitor/core";
import { useCallback, useEffect, useRef, useState } from "react";

/** A system screen a message's photos and videos come from. */
export type MediaSource = "photo" | "video" | "library";

/**
 * The mobile apps' native side of the system's own screens for photos and videos
 * (`AspenMediaPlugin`, in each): the photo picker, which hands the app only what was chosen and
 * so needs no permission to read the device's photos, and the camera. Each thing chosen or
 * taken is copied into the app's cache and answered as a `file:` URL the page reads it from and
 * then `release`s.
 */
interface AspenMediaPlugin {
  sources(): Promise<Record<MediaSource, boolean>>;
  /**
   * Opens `source`'s screen, for at most `limit` things where the user chooses. Answers none
   * when the user backed out; refuses with code `cameraDenied` where the app may not use the
   * camera.
   */
  choose(options: { source: MediaSource; limit: number }): Promise<{ items: NativeMedia[] }>;
  release(options: { url: string }): Promise<void>;
}

interface NativeMedia {
  url: string;
  /** Its name, or empty where the system gives none worth keeping. */
  name: string;
  mimeType: string;
  /** The copy's length, which the page reads it whole to. */
  byteSize: number;
  /** When it was taken, in milliseconds since the epoch, where the system says. */
  takenAt?: number;
}

/**
 * Something chosen or taken. A file without a name (`named` false) is to be named for when it
 * was taken: `takenAt`, which is now for what the camera has just taken.
 */
export interface ChosenMedia {
  file: File;
  named: boolean;
  takenAt: Date;
}

const AspenMedia = registerPlugin<AspenMediaPlugin>("AspenMedia");

const NONE: ReadonlySet<MediaSource> = new Set();

let known: Promise<ReadonlySet<MediaSource>> | null = null;

/**
 * The sources this device's system offers a screen of its own for, none outside the mobile apps:
 * a browser's file picker cannot be asked for a photo picker or the camera alone everywhere, nor
 * asked whether it has them. A device's cameras and picker do not come and go while the app
 * runs, so it is asked once.
 */
export function mediaSources(): Promise<ReadonlySet<MediaSource>> {
  if (!Capacitor.isNativePlatform() || !Capacitor.isPluginAvailable("AspenMedia")) {
    return Promise.resolve(NONE);
  }
  known ??= AspenMedia.sources().then(
    (offered) =>
      new Set((["photo", "video", "library"] as const).filter((source) => offered[source])),
  );
  return known;
}

/** Whether `error` is the native side's refusal because the app may not use the camera. */
export function isCameraDenied(error: unknown): boolean {
  return (
    typeof error === "object" && error !== null && "code" in error && error.code === "cameraDenied"
  );
}

/**
 * Opens `source`'s screen and answers what was chosen or taken there, read into the page; none
 * when the user backed out. Every copy is released, read or not.
 */
export async function chooseMedia(source: MediaSource, limit: number): Promise<ChosenMedia[]> {
  const { items } = await AspenMedia.choose({ source, limit });
  try {
    return await Promise.all(
      items.map(async ({ url, name, mimeType, byteSize, takenAt }) => {
        // Asked as a range, Capacitor's file server answers an HTTP response on iOS for every
        // type; asked plainly, a video's answer there is no HTTP response, which `fetch` refuses.
        // Android's answers a range with the whole file from its start, so the range asked is
        // the whole file.
        const response = await fetch(Capacitor.convertFileSrc(url), {
          headers: { Range: "bytes=0-" },
        });
        if (!response.ok) {
          throw new Error(`a chosen file could not be read (${String(response.status)})`);
        }
        const blob = await response.blob();
        if (blob.size !== byteSize) {
          throw new Error(
            `a chosen file was read as ${String(blob.size)} bytes of ${String(byteSize)}`,
          );
        }
        return {
          file: new File([blob], name, { type: mimeType }),
          named: name !== "",
          takenAt: takenAt === undefined ? new Date() : new Date(takenAt),
        };
      }),
    );
  } finally {
    for (const { url } of items) {
      void AspenMedia.release({ url }).catch(() => undefined);
    }
  }
}

/**
 * The sources this device offers (none until known), and `choose`, which opens one and hands
 * what came of it to `take`, or why nothing could to `fail`. Neither is called once the caller
 * is gone.
 */
export function useMediaChooser(
  take: (chosen: ChosenMedia[]) => void,
  fail: (error: unknown) => void,
): {
  sources: ReadonlySet<MediaSource>;
  choose: (source: MediaSource, limit: number) => void;
} {
  const [sources, setSources] = useState(NONE);
  const latest = useRef({ take, fail });
  useEffect(() => {
    latest.current = { take, fail };
  });
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    // A device whose sources cannot be learned is offered none.
    mediaSources().then(
      (known) => {
        if (live.current) {
          setSources(known);
        }
      },
      () => undefined,
    );
    return () => {
      live.current = false;
    };
  }, []);
  const choose = useCallback((source: MediaSource, limit: number) => {
    chooseMedia(source, limit).then(
      (chosen) => {
        if (live.current) {
          latest.current.take(chosen);
        }
      },
      (error: unknown) => {
        if (live.current) {
          latest.current.fail(error);
        }
      },
    );
  }, []);
  return { sources, choose };
}
