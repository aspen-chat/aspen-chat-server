/**
 * What a voice call needs from the platform's media: the call's peer connection, capture and
 * playback (`VoiceMedia`), and the shares and sounds sent from outside the browser, which answer
 * the voice server's offers of a connection of their own. `browserMedia.ts` implements
 * `VoiceMedia` for a browser, and tests pass a fake.
 */

import type { Section } from "./generated/voiceSignal";
import type { DeviceChoice } from "./preferences";

/** An offer of the capture connection, the one a sender outside the browser holds. */
export interface CaptureOffer {
  readonly revision: number;
  readonly sdp: string;
  /** What each media section carries: one `sending` section per source the sender sends. */
  readonly sections: readonly Section[];
}

/**
 * A sender outside the browser, such as the desktop shell's capture helper, holding the capture
 * connection to the voice server: the call passes it every offer of that connection, the first
 * starting it, and sends the voice server its answers, until `stop`.
 */
export interface ExternalSender {
  answer(offer: CaptureOffer): Promise<string>;
  stop(): void;
}

/**
 * A share sent from outside the browser, such as the desktop shell's game capture: its picture,
 * and its own sound when `audio` says it brings any.
 */
export interface ExternalShare extends ExternalSender {
  readonly audio: boolean;
}

/**
 * Sound for a browser screen share sent from outside the browser, such as one application's
 * audio captured by the desktop shell. It stands in for any sound the browser captured with the
 * picture.
 */
export type ExternalAudio = ExternalSender;

/** What `getDisplayMedia` gave: the picture, and the sound that came with it when the browser offered any. */
export interface ScreenCapture {
  readonly video: MediaStreamTrack;
  readonly audio: MediaStreamTrack | null;
}

/** A transceiver of the call's peer connection, narrowed to what the call needs. */
export interface CallTransceiver {
  readonly mid: string | null;
  direction: RTCRtpTransceiverDirection;
  readonly sender: {
    readonly track: MediaStreamTrack | null;
    replaceTrack(track: MediaStreamTrack | null): Promise<void>;
    getParameters(): RTCRtpSendParameters;
    setParameters(parameters: RTCRtpSendParameters): Promise<void>;
  };
  readonly receiver: { readonly track: MediaStreamTrack };
}

/**
 * The call's peer connection, narrowed to what the call needs. The voice server makes every
 * offer, so the call only ever answers.
 */
export interface CallConnection {
  /** `new`, `connecting`, `connected`, `disconnected`, `failed`, or `closed`. */
  readonly connectionState: RTCPeerConnectionState;
  readonly localDescription: { readonly sdp: string } | null;
  setRemoteDescription(description: RTCSessionDescriptionInit): Promise<void>;
  createAnswer(): Promise<RTCSessionDescriptionInit>;
  setLocalDescription(description: RTCSessionDescriptionInit): Promise<void>;
  getTransceivers(): CallTransceiver[];
  addEventListener(type: "connectionstatechange", listener: () => void): void;
  close(): void;
}

/**
 * Whether this browser can capture a screen: mobile browsers cannot, and a page not served over
 * HTTPS has no media devices at all. Answered `false` outside a browser.
 */
export function screenCaptureAvailable(): boolean {
  return (
    typeof navigator !== "undefined" &&
    "mediaDevices" in navigator &&
    typeof navigator.mediaDevices.getDisplayMedia === "function"
  );
}

/** What the call needs from the browser: its peer connection, media capture, and playback. */
export interface VoiceMedia {
  /** Whether `getScreen` can capture a screen here (`screenCaptureAvailable` in a browser). */
  readonly hasScreenCapture: boolean;
  createConnection(): Promise<CallConnection>;
  /**
   * Opens the microphone the choice names, or the system's default, sent at `gain` (as
   * `setInputVolume` takes it) until `setInputVolume` changes it.
   */
  getMicrophone(choice: DeviceChoice, gain: number): Promise<MediaStreamTrack>;
  /** Scales what the open microphone sends: 1 as captured, 0 silent, up to 2. */
  setInputVolume(gain: number): void;
  /** Opens a camera, the chosen one when it is present. */
  getCamera(choice: DeviceChoice): Promise<MediaStreamTrack>;
  /** Routes everything played to the speaker the choice names, or the system's default. */
  setOutput(choice: DeviceChoice): Promise<void>;
  /** Asks the user for a screen, window, or tab to share; rejects when they decline. */
  getScreen(): Promise<ScreenCapture>;
  /** Plays a received audio track under `id`, which `stop` and `setVolume` name it by. */
  play(id: string, track: MediaStreamTrack): void;
  stop(id: string): void;
  /** Scales what a playing track is heard at: 1 is as sent, 0 silent, up to 2. */
  setVolume(id: string, gain: number): void;
}
