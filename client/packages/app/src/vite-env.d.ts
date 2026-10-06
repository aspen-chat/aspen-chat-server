/// <reference types="vite/client" />

/** Every package Aspen ships, with its licenses (`attributions/plugin.ts`). */
declare module "virtual:attributions" {
  import type { Attributions } from "@/features/about/attributionTypes";
  const attributions: Attributions;
  export default attributions;
}

/** This build of the app: its version and commit (`attributions/plugin.ts`). */
declare module "virtual:build-info" {
  import type { BuildInfo } from "@/features/about/attributionTypes";
  const build: BuildInfo;
  export default build;
}

interface ImportMetaEnv {
  /**
   * `1` builds the message list with its scroll diagnostics
   * (`features/messages/scrollDiagnostics.ts`), for finding where a view jumps on a device.
   */
  readonly VITE_SCROLL_DEBUG?: string;
}

/** Where an external sender delivers SRTP, as the voice server describes it. */
interface RtpTargetShape {
  ip: string;
  port: number;
  ssrc: number;
  payloadType: number;
  srtpCryptoSuite: string;
  srtpKeyBase64: string;
}

/** An audio source and the applications it can be pointed at, as the capture helper lists them. */
interface AudioKindShape {
  kind: string;
  property: string;
  targets: { name: string; pid: number | null; settings: Record<string, unknown> }[] | null;
}

/** One application's sound for the capture helper to send as Opus. */
interface AudioOptionsShape {
  kind: string;
  settings?: string;
  rtp: RtpTargetShape;
  bitrateKbps?: number;
}

/** Injected by the Electron preload script; absent in browsers and Capacitor. */
interface AspenDesktopBridge {
  readonly platform: NodeJS.Platform;
  readonly versions: { electron: string; chrome: string };
  /** Screen sharing's picker: the sources the main process lists, and the page's answer. */
  readonly displayPicker: {
    onPick(listener: (request: unknown) => void): () => void;
    choose(choice: { id: string | null; systemAudio: boolean }): void;
  };
  /** Passkey ceremonies in the system browser (`packages/desktop/src/main/passkeyHandoff.ts`). */
  readonly passkeyHandoff: {
    prepare(): Promise<{ id: string; returnTo: string }>;
    open(id: string, url: string): Promise<{ ceremony: string; outcome: "done" | "cancelled" }>;
    dispose(id: string): void;
  };
  /**
   * `aspen://app/…` links the system handed the app (`packages/desktop/src/main/appLinks.ts`):
   * `ready` returns those that waited, and `onOpen` hears each later one.
   */
  readonly appLinks: {
    ready(): Promise<string[]>;
    onOpen(listener: (url: string) => void): () => void;
  };
  /**
   * The window's zoom (`packages/desktop/src/main/zoom.ts`): the factor it keeps, setting it,
   * and Ctrl + and Ctrl − (with Ctrl 0 and Ctrl with the wheel), which the main process takes
   * from the page and hands back as steps for `theme/zoom.ts` to make.
   */
  readonly zoom: {
    get(): Promise<number>;
    set(factor: number): Promise<void>;
    onStep(listener: (step: 1 | -1 | 0) => void): () => void;
  };
  /** Chromium's notices (`packages/desktop/src/main/chromiumNotices.ts`); whether they opened. */
  readonly chromiumNotices: {
    open(): Promise<boolean>;
  };
  /** Game capture through libobs; the shape is `GameCaptureBridge` in `features/voice/gameCapture.ts`. */
  readonly gameCapture: {
    kinds(): Promise<{
      kinds: {
        kind: string;
        property: string;
        targets: { name: string; value: string }[];
        audio: AudioKindShape | null;
      }[];
      applicationAudio: AudioKindShape | null;
      testMedia: string | null;
    }>;
    start(options: {
      kind: string;
      settings?: string;
      width?: number;
      height?: number;
      fps?: number;
      bitrateKbps?: number;
      rtp: RtpTargetShape;
      audio?: AudioOptionsShape;
    }): Promise<void>;
    /** Captures one application's sound alone, the picture coming from the browser's screen share. */
    startAudio(audio: AudioOptionsShape): Promise<void>;
    stop(): Promise<void>;
    /** The capture ended without being asked to: the helper process died. */
    onEnded(listener: () => void): () => void;
  };
}

interface Window {
  readonly aspenDesktop?: AspenDesktopBridge;
}
