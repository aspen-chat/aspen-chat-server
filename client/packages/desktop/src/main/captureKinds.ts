/**
 * What the capture helper may be asked to start: only sources it listed itself, so a renderer
 * cannot have libobs open a source of any other kind (a browser source, a file, a camera)
 * through the bridge. The last listing (`offeredKinds`) is what `startOffered` and
 * `audioOffered` check against; before one, nothing is offered.
 *
 * This module imports nothing from Electron, so its tests run under plain Node.
 */

/** The kind of the development test pattern: libobs's media source, playing one file. */
export const TEST_PATTERN_KIND = "ffmpeg_source";

export interface Offered {
  /** The picture kinds listed. */
  readonly video: ReadonlySet<string>;
  /** The audio kinds listed beside a picture kind, for its window's application. */
  readonly windowAudio: ReadonlySet<string>;
  /** The audio kind listed for an application's sound alone. */
  readonly applicationAudio: string | null;
  /** The file the test pattern plays, when the shell offers one. */
  readonly testMedia: string | null;
}

export const NOTHING_OFFERED: Offered = {
  video: new Set(),
  windowAudio: new Set(),
  applicationAudio: null,
  testMedia: null,
};

export function offeredKinds(listing: {
  kinds: readonly { kind: string; audio: { kind: string } | null }[];
  applicationAudio: { kind: string } | null;
  testMedia: string | null;
}): Offered {
  return {
    video: new Set(listing.kinds.map((kind) => kind.kind)),
    windowAudio: new Set(
      listing.kinds.flatMap((kind) => (kind.audio === null ? [] : [kind.audio.kind])),
    ),
    applicationAudio: listing.applicationAudio?.kind ?? null,
    testMedia: listing.testMedia,
  };
}

/**
 * Whether a capture of a picture (and its sound) was offered: a listed picture kind with a
 * listed audio kind, or the test pattern playing the very file offered, with its own sound.
 */
export function startOffered(
  offered: Offered,
  options: { kind: string; settings?: string | undefined; audio?: { kind: string } | undefined },
): boolean {
  if (options.kind === TEST_PATTERN_KIND && offered.testMedia !== null) {
    return (
      playsFile(options.settings, offered.testMedia) &&
      (options.audio === undefined || options.audio.kind === "")
    );
  }
  return (
    offered.video.has(options.kind) &&
    (options.audio === undefined || offered.windowAudio.has(options.audio.kind))
  );
}

/** Whether a capture of an application's sound alone was offered. */
export function audioOffered(offered: Offered, audio: { kind: string }): boolean {
  return offered.applicationAudio !== null && audio.kind === offered.applicationAudio;
}

function playsFile(settings: string | undefined, file: string): boolean {
  try {
    const parsed: unknown = JSON.parse(settings ?? "{}");
    return (
      typeof parsed === "object" &&
      parsed !== null &&
      (parsed as { local_file?: unknown }).local_file === file
    );
  } catch {
    return false;
  }
}
