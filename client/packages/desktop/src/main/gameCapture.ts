import { app, ipcMain, type WebContents } from "electron";
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";

/**
 * Game capture through libobs, for the renderer. The helper `aspen-obs-capture` (the Rust
 * crate in `native/obs-capture`) captures a game or window, encodes it to H.264, and sends it
 * as SRTP straight to the voice server; the shell never touches the video. On Linux it captures
 * only one application's sound (`startAudio`), since there the picture comes from the
 * browser's own screen share. The helper is a
 * process of its own, spawned on first use and spoken to over its pipes: one JSON request per
 * line on its stdin, one JSON reply per line on its stdout (the requests are described at the
 * top of the crate's `main.rs`). One capture runs at a time, for one renderer. Without the
 * helper on disk (a build without libobs) the bridge reports no capture kinds and the app shows
 * no game option.
 */

export interface CaptureTarget {
  name: string;
  value: string;
}

/** An application playing sound, where the platform lists them (Linux, through PipeWire). */
export interface AudioTarget {
  name: string;
  pid: number | null;
  settings: Record<string, unknown>;
}

/**
 * An audio source: pointed at a captured window's application through `property`, which takes
 * the window's value, when `targets` is null; otherwise at one of the applications listed in
 * `targets`, chosen on its own.
 */
export interface AudioKind {
  kind: string;
  property: string;
  targets: AudioTarget[] | null;
}

export interface CaptureKind {
  kind: string;
  property: string;
  targets: CaptureTarget[];
  /** The audio source for the captured window's application, where this platform has one. */
  audio: AudioKind | null;
}

/**
 * What the renderer learns about capture: the game capture kinds, the applications whose sound
 * can be captured on its own (Linux, where the picture comes from the browser's screen share),
 * and in development a clip to test with.
 */
export interface CaptureCatalogue {
  kinds: CaptureKind[];
  applicationAudio: AudioKind | null;
  /** A media file to capture as a test pattern, named by `ASPEN_TEST_MEDIA`; development only. */
  testMedia: string | null;
}

/** Where the helper sends its SRTP, as the voice server answered `produceRtp`. */
export interface RtpTarget {
  ip: string;
  port: number;
  ssrc: number;
  payloadType: number;
  srtpCryptoSuite: string;
  srtpKeyBase64: string;
}

/** The sound of one application, sent as Opus to its RTP target. */
export interface AudioOptions {
  kind: string;
  settings?: string;
  rtp: RtpTarget;
  bitrateKbps?: number;
}

export interface StartOptions {
  kind: string;
  settings?: string;
  width?: number;
  height?: number;
  fps?: number;
  bitrateKbps?: number;
  rtp: RtpTarget;
  /** The application's audio: its source and where to send it, when wanted and available. */
  audio?: AudioOptions;
}

type Reply =
  | { type: "kinds"; id: number; kinds: CaptureKind[]; applicationAudio: AudioKind | null }
  | { type: "done"; id: number }
  | { type: "failed"; id: number; reason: string };

export const CAPTURE_KINDS_CHANNEL = "voice:capture-kinds";
export const CAPTURE_START_CHANNEL = "voice:capture-start";
export const CAPTURE_START_AUDIO_CHANNEL = "voice:capture-start-audio";
export const CAPTURE_STOP_CHANNEL = "voice:capture-stop";
/** Sent to the renderer whose capture ended without it asking: the helper died. */
export const CAPTURE_ENDED_CHANNEL = "voice:capture-ended";

function helperPath(): string | null {
  const name = process.platform === "win32" ? "aspen-obs-capture.exe" : "aspen-obs-capture";
  // Built by `pnpm build:native` next to the crate; packaged builds carry it as a resource.
  const candidates = [join(process.resourcesPath, name), join(app.getAppPath(), "native", name)];
  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

/**
 * The libobs the Windows build ships (`scripts/fetch-libobs.mjs`): `bin/64bit` with the
 * libraries, `obs-plugins/64bit` with the modules, and `data`. Packaged builds carry it as a
 * resource; in development it is beside the crate. Elsewhere `null`, and the helper uses the
 * paths it was built with.
 */
function bundledLibobs(): string | null {
  if (process.platform !== "win32") {
    return null;
  }
  const candidates = [
    join(process.resourcesPath, "libobs"),
    join(app.getAppPath(), "native", "libobs"),
  ];
  return (
    candidates.find((candidate) => existsSync(join(candidate, "bin", "64bit", "obs.dll"))) ?? null
  );
}

class CaptureHost {
  #child: ChildProcess | null = null;
  #owner: WebContents | null = null;
  #nextId = 1;
  /** Where the helper's libobs modules and data are, when the build ships them. */
  #dirs: { pluginDir?: string; dataDir?: string } = {};
  readonly #pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (reason: Error) => void }
  >();

  #process(): ChildProcess | null {
    if (this.#child !== null) {
      return this.#child;
    }
    const helper = helperPath();
    if (helper === null) {
      return null;
    }
    // With a bundled libobs, the helper finds obs.dll and what it loads through the PATH, and
    // is told where the modules and their data are with every request.
    const libobs = bundledLibobs();
    const env =
      libobs === null
        ? process.env
        : { ...process.env, PATH: `${join(libobs, "bin", "64bit")};${process.env.PATH ?? ""}` };
    this.#dirs =
      libobs === null
        ? {}
        : { pluginDir: join(libobs, "obs-plugins", "64bit"), dataDir: join(libobs, "data") };
    const child = spawn(helper, [], { stdio: ["pipe", "pipe", "inherit"], env });
    // stdout is a pipe, so it is present; readline turns it into whole lines.
    createInterface({ input: child.stdout }).on("line", (line) => {
      this.#onReply(JSON.parse(line) as Reply);
    });
    child.on("exit", (code) => {
      if (this.#child === child) {
        this.#child = null;
        for (const { reject } of this.#pending.values()) {
          reject(new Error(`the capture helper exited (${String(code)})`));
        }
        this.#pending.clear();
        const owner = this.#owner;
        this.#owner = null;
        if (owner !== null && !owner.isDestroyed()) {
          owner.send(CAPTURE_ENDED_CHANNEL);
        }
      }
    });
    this.#child = child;
    return child;
  }

  #onReply(reply: Reply): void {
    const pending = this.#pending.get(reply.id);
    this.#pending.delete(reply.id);
    if (reply.type === "failed") {
      pending?.reject(new Error(reply.reason));
    } else {
      pending?.resolve(
        reply.type === "kinds"
          ? { kinds: reply.kinds, applicationAudio: reply.applicationAudio }
          : undefined,
      );
    }
  }

  #request(request: Record<string, unknown>): Promise<unknown> {
    const child = this.#process();
    if (child === null) {
      return Promise.reject(new Error("game capture is not available in this build"));
    }
    const id = this.#nextId++;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      child.stdin?.write(`${JSON.stringify({ ...request, id })}\n`);
    });
  }

  async kinds(): Promise<CaptureCatalogue> {
    const testMedia = app.isPackaged ? null : (process.env.ASPEN_TEST_MEDIA ?? null);
    if (helperPath() === null) {
      return { kinds: [], applicationAudio: null, testMedia };
    }
    const listed = (await this.#request({ type: "kinds", ...this.#dirs })) as Omit<
      CaptureCatalogue,
      "testMedia"
    > & {
      /** Whether the helper can capture a picture at all (a build with libobs). */
      pictures: boolean;
    };
    // The test pattern is a picture, through libobs's media source.
    return {
      kinds: listed.kinds,
      applicationAudio: listed.applicationAudio,
      testMedia: listed.pictures ? testMedia : null,
    };
  }

  async start(sender: WebContents, options: StartOptions): Promise<void> {
    await this.#request({ type: "start", options: { ...options, ...this.#dirs } });
    this.#own(sender);
  }

  /** Captures one application's sound alone, the picture coming from the browser's screen share. */
  async startAudio(sender: WebContents, audio: AudioOptions): Promise<void> {
    await this.#request({ type: "startAudio", options: { audio, ...this.#dirs } });
    this.#own(sender);
  }

  /** Makes `sender` the capture's owner, stopping it when that renderer goes away. */
  #own(sender: WebContents): void {
    this.#owner = sender;
    sender.once("destroyed", () => {
      if (this.#owner === sender) {
        void this.stop(sender);
      }
    });
  }

  async stop(sender: WebContents): Promise<void> {
    if (this.#owner !== sender) {
      return;
    }
    this.#owner = null;
    if (this.#child !== null) {
      await this.#request({ type: "stop" });
    }
  }

  /** Ends the helper: it stops any capture and shuts libobs down on its way out. */
  shutdown(): void {
    const child = this.#child;
    this.#child = null;
    this.#owner = null;
    if (child !== null) {
      child.stdin?.write(`${JSON.stringify({ type: "shutdown" })}\n`);
      child.stdin?.end();
    }
  }
}

/** Answers the renderer's capture requests for the life of the app. */
export function serveGameCapture(): void {
  const host = new CaptureHost();
  ipcMain.handle(CAPTURE_KINDS_CHANNEL, async (): Promise<CaptureCatalogue> => {
    try {
      return await host.kinds();
    } catch (error) {
      console.error("libobs could not list capture kinds", error);
      return { kinds: [], applicationAudio: null, testMedia: null };
    }
  });
  ipcMain.handle(CAPTURE_START_CHANNEL, (event, options: StartOptions) =>
    host.start(event.sender, options),
  );
  ipcMain.handle(CAPTURE_START_AUDIO_CHANNEL, (event, audio: AudioOptions) =>
    host.startAudio(event.sender, audio),
  );
  ipcMain.handle(CAPTURE_STOP_CHANNEL, (event) => host.stop(event.sender));
  app.on("will-quit", () => {
    host.shutdown();
  });
}
