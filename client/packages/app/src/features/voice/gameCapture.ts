import type { ExternalAudio, ExternalShare, ExternalTargets, RtpTarget } from "@aspen/protocol";

/**
 * Game capture on the desktop shell. The Electron main process runs the libobs helper (see
 * `packages/desktop/native/obs-capture`), which captures a game or window, encodes it to
 * H.264, and sends it as SRTP straight to the voice server; the renderer only asks for the
 * producer and tells the helper where to send. On Linux the helper captures one application's
 * sound only, and the picture is the browser's own screen share. Nothing here exists in a
 * browser: `gameCaptureBridge()` is `null` there.
 */

export interface CaptureTarget {
  readonly name: string;
  readonly value: string;
}

/** An application playing sound, as the platform lists them (Linux, through PipeWire). */
export interface AudioTarget {
  /** The application's name; empty when it gave none. */
  readonly name: string;
  readonly pid: number | null;
  /** The audio source's settings that capture it. */
  readonly settings: Readonly<Record<string, unknown>>;
}

/**
 * An audio source: pointed at a captured window's application through `property`, which takes
 * the window's value, when `targets` is null; otherwise at one of `targets`, chosen on its own.
 */
export interface AudioKind {
  readonly kind: string;
  readonly property: string;
  readonly targets: readonly AudioTarget[] | null;
}

export interface CaptureKind {
  /** The libobs source id, such as `game_capture`. */
  readonly kind: string;
  /** The setting that names the window. */
  readonly property: string;
  readonly targets: readonly CaptureTarget[];
  /** The audio source for the captured window's application, where this platform has one. */
  readonly audio: AudioKind | null;
}

/**
 * What the shell can capture: the game capture kinds (none on Linux), the applications whose
 * sound can be captured on its own (Linux only), and in development a clip to test with.
 */
export interface CaptureCatalogue {
  readonly kinds: readonly CaptureKind[];
  readonly applicationAudio: AudioKind | null;
  readonly testMedia: string | null;
}

/** One application's sound for the helper to send as Opus. */
export interface AudioCaptureStart {
  readonly kind: string;
  readonly settings?: string;
  readonly rtp: RtpTarget;
  readonly bitrateKbps?: number;
}

export interface CaptureStart {
  readonly kind: string;
  readonly settings?: string;
  readonly width?: number;
  readonly height?: number;
  readonly fps?: number;
  readonly bitrateKbps?: number;
  readonly rtp: RtpTarget;
  readonly audio?: AudioCaptureStart;
}

/** The preload bridge's game capture surface. */
export interface GameCaptureBridge {
  kinds(): Promise<CaptureCatalogue>;
  start(options: CaptureStart): Promise<void>;
  /** Captures one application's sound alone. */
  startAudio(audio: AudioCaptureStart): Promise<void>;
  stop(): Promise<void>;
  /** The capture ended without being asked to: the helper process died. */
  onEnded(listener: () => void): () => void;
}

export function gameCaptureBridge(): GameCaptureBridge | null {
  return window.aspenDesktop?.gameCapture ?? null;
}

/**
 * What to capture: a source kind and the settings that point it at one target, and the audio
 * source that captures the same application's sound when the platform has one. An audio kind
 * of `""` means no source of its own: the sound the video source itself produces, as a media
 * file does.
 */
export interface CaptureChoice {
  readonly kind: string;
  readonly settings: Record<string, unknown>;
  readonly audio: { readonly kind: string; readonly settings: Record<string, unknown> } | null;
}

/**
 * The settings that make a capture kind capture one window, in each source's own vocabulary:
 * `game_capture` wants a capture mode along with the window, and macOS's `screen_capture`
 * captures an application (type 2) so its audio source can name the same one. The audio
 * source, where the platform has one, is pointed at the same window.
 */
export function captureChoice(kind: CaptureKind, target: CaptureTarget): CaptureChoice {
  const settings: Record<string, unknown> = { [kind.property]: target.value };
  if (kind.kind === "game_capture") {
    settings.capture_mode = "window";
  } else if (kind.kind === "screen_capture") {
    settings.type = 2;
  }
  const audio =
    kind.audio === null
      ? null
      : { kind: kind.audio.kind, settings: { [kind.audio.property]: target.value } };
  return { kind: kind.kind, settings, audio };
}

/** How often the dialog lists the applications playing sound again while it is open. */
export const AUDIO_REFRESH_MS = 500;

/** The audio choice that sends no application's sound. */
export const NO_AUDIO = "none";

/**
 * A key naming one application across refreshes of the list: its process, or its name when it
 * gave no process id, as the helper groups them.
 */
export function audioTargetKey(target: AudioTarget): string {
  return target.pid === null ? `name:${target.name}` : `pid:${String(target.pid)}`;
}

/** Which application's sound is selected, and whether the sharer picked it themself. */
export interface AudioChoice {
  readonly key: string;
  readonly chosen: boolean;
}

/**
 * The selection once the applications playing sound are listed again. Until the sharer picks,
 * the only application playing is preselected, being the likely game, and none when several
 * or none play. A pick stands while its application is listed and falls to no audio when it
 * goes, rather than moving to some other application's sound.
 */
export function refreshAudioChoice(
  choice: AudioChoice,
  targets: readonly AudioTarget[],
): AudioChoice {
  if (!choice.chosen) {
    const only = targets.length === 1 ? targets[0] : undefined;
    return { key: only === undefined ? NO_AUDIO : audioTargetKey(only), chosen: false };
  }
  const listed =
    choice.key === NO_AUDIO || targets.some((target) => audioTargetKey(target) === choice.key);
  return listed ? choice : { key: NO_AUDIO, chosen: true };
}

/** Whether two listings name the same applications with the same settings, in the same order. */
export function sameAudioTargets(
  a: readonly AudioTarget[] | null,
  b: readonly AudioTarget[] | null,
): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * A test pattern for trying the pipeline without a game: the media file the shell names,
 * looping through libobs's media source, picture and sound both.
 */
export function testPattern(testMedia: string): CaptureChoice {
  return {
    kind: "ffmpeg_source",
    settings: { is_local_file: true, local_file: testMedia, looping: true, hw_decode: false },
    audio: { kind: "", settings: {} },
  };
}

/**
 * A game capture as an external share for `VoiceCall.startExternalScreenShare`: the call
 * obtains the RTP producer and hands its target to `start`, which starts the helper sending;
 * `stop` ends the capture. `onEnded` is called when the helper dies mid-share, so the caller
 * can end the share.
 */
export function gameCaptureShare(
  bridge: GameCaptureBridge,
  choice: CaptureChoice,
  withAudio: boolean,
  onEnded: () => void,
): ExternalShare {
  let unsubscribe: (() => void) | null = null;
  const audio = withAudio ? choice.audio : null;
  return {
    audio: audio !== null,
    async start(targets: ExternalTargets) {
      unsubscribe = bridge.onEnded(onEnded);
      try {
        await bridge.start({
          kind: choice.kind,
          settings: JSON.stringify(choice.settings),
          width: 1920,
          height: 1080,
          fps: 60,
          bitrateKbps: 6000,
          rtp: targets.video,
          ...(audio === null || targets.audio === null
            ? {}
            : {
                audio: {
                  kind: audio.kind,
                  settings: JSON.stringify(audio.settings),
                  rtp: targets.audio,
                },
              }),
        });
      } catch (error) {
        unsubscribe();
        unsubscribe = null;
        throw error;
      }
    },
    stop() {
      unsubscribe?.();
      unsubscribe = null;
      void bridge.stop();
    },
  };
}

/**
 * One application's sound, captured by the helper, as the external audio of a browser screen
 * share (`VoiceCall.startScreenShare({ audio })`): the call obtains the RTP producer and hands
 * its target to `start`; `stop` ends the capture. `onEnded` is called when the helper dies
 * mid-share, so the caller can end the share.
 */
export function applicationAudioShare(
  bridge: GameCaptureBridge,
  kind: AudioKind,
  application: AudioTarget,
  onEnded: () => void,
): ExternalAudio {
  let unsubscribe: (() => void) | null = null;
  return {
    async start(target: RtpTarget) {
      unsubscribe = bridge.onEnded(onEnded);
      try {
        await bridge.startAudio({
          kind: kind.kind,
          settings: JSON.stringify(application.settings),
          rtp: target,
        });
      } catch (error) {
        unsubscribe();
        unsubscribe = null;
        throw error;
      }
    },
    stop() {
      unsubscribe?.();
      unsubscribe = null;
      void bridge.stop();
    },
  };
}
