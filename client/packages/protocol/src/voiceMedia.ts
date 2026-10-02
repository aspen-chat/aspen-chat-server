/**
 * What a voice call needs from the platform's media: the mediasoup device and its transports,
 * capture and playback (`VoiceMedia`), and the shares and sounds produced outside the browser
 * that the call hands RTP targets to. `browserMedia.ts` implements `VoiceMedia` for a browser,
 * and tests pass a fake.
 */

import type { DeviceChoice } from "./preferences";

/** Where and how an external sender delivers SRTP for a producer the voice server made for it. */
export interface RtpTarget {
  readonly ip: string;
  readonly port: number;
  readonly ssrc: number;
  readonly payloadType: number;
  readonly srtpCryptoSuite: string;
  readonly srtpKeyBase64: string;
}

/** Where an external share sends: the picture, and the sound when the share carries any. */
export interface ExternalTargets {
  readonly video: RtpTarget;
  readonly audio: RtpTarget | null;
}

/**
 * A share produced outside the browser, such as the desktop shell's game capture: the call
 * asks the voice server for an RTP producer per stream, then `start` sends to them until
 * `stop`. `audio` says whether the share brings sound of its own, which needs a producer too.
 */
export interface ExternalShare {
  readonly audio: boolean;
  start(targets: ExternalTargets): Promise<void>;
  stop(): void;
}

/**
 * Sound for a browser screen share that comes from outside the browser, such as one
 * application's audio captured by the desktop shell: the call asks the voice server for an RTP
 * producer, then `start` sends to it until `stop`. It stands in for any sound the browser
 * captured with the picture.
 */
export interface ExternalAudio {
  start(target: RtpTarget): Promise<void>;
  stop(): void;
}

/** What `getDisplayMedia` gave: the picture, and the sound that came with it when the browser offered any. */
export interface ScreenCapture {
  readonly video: MediaStreamTrack;
  readonly audio: MediaStreamTrack | null;
}

/** A transport as mediasoup-client models it, narrowed to what the call needs. */
export interface VoiceTransport {
  /** The ICE and DTLS state: `new`, `connecting`, `connected`, `disconnected`, `failed`, or `closed`. */
  readonly connectionState: string;
  on(event: "connectionstatechange", handler: (state: string) => void): unknown;
  on(
    event: "connect",
    handler: (
      params: { dtlsParameters: unknown },
      callback: () => void,
      errback: (error: Error) => void,
    ) => void,
  ): unknown;
  on(
    event: "produce",
    handler: (
      params: { kind: string; rtpParameters: unknown; appData: Record<string, unknown> },
      callback: (result: { id: string }) => void,
      errback: (error: Error) => void,
    ) => void,
  ): unknown;
  produce(options: {
    track: MediaStreamTrack;
    appData: Record<string, unknown>;
    encodings?: { maxBitrate?: number; maxFramerate?: number }[];
    codecOptions?: {
      opusStereo?: boolean;
      opusDtx?: boolean;
      opusMaxAverageBitrate?: number;
      videoGoogleStartBitrate?: number;
    };
  }): Promise<{
    id: string;
    close(): void;
    replaceTrack(options: { track: MediaStreamTrack }): Promise<void>;
  }>;
  consume(options: {
    id: string;
    producerId: string;
    kind: "audio" | "video";
    rtpParameters: unknown;
  }): Promise<{ id: string; track: MediaStreamTrack; close(): void }>;
  close(): void;
}

/** A mediasoup-client device, narrowed to what the call needs. */
export interface VoiceDevice {
  load(options: { routerRtpCapabilities: unknown }): Promise<void>;
  readonly rtpCapabilities: unknown;
  createSendTransport(params: TransportParams): VoiceTransport;
  createRecvTransport(params: TransportParams): VoiceTransport;
}

export interface TransportParams {
  id: string;
  iceParameters: unknown;
  iceCandidates: unknown;
  dtlsParameters: unknown;
}

/** What the call needs from the browser: media capture, the mediasoup device, and playback. */
export interface VoiceMedia {
  createDevice(): Promise<VoiceDevice>;
  /** Opens the microphone the choice names, or the system's default. */
  getMicrophone(choice: DeviceChoice): Promise<MediaStreamTrack>;
  /** Opens a camera, the chosen one when it is present. */
  getCamera(choice: DeviceChoice): Promise<MediaStreamTrack>;
  /** Routes everything played to the speaker the choice names, or the system's default. */
  setOutput(choice: DeviceChoice): Promise<void>;
  /** Asks the user for a screen, window, or tab to share; rejects when they decline. */
  getScreen(): Promise<ScreenCapture>;
  /** Plays a remote track; called once per consumer. */
  play(consumerId: string, track: MediaStreamTrack): void;
  stop(consumerId: string): void;
  /** Scales what a playing consumer is heard at: 1 is as sent, 0 silent, up to 2. */
  setVolume(consumerId: string, gain: number): void;
}
