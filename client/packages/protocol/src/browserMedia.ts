/**
 * The browser's side of a voice call: the microphone through `getUserMedia`, mediasoup-client's
 * `Device` for the WebRTC transports, and playback through hidden `<audio>` elements, one per
 * consumer, appended to the document so autoplay policy sees a playing element per track.
 */

import { Device } from "mediasoup-client";
import { DEFAULT_DEVICE, type DeviceChoice, resolveDevice } from "./preferences";
import { CameraError, type ScreenCapture, type VoiceDevice, type VoiceMedia } from "./voice";

/** The device id a choice means right now, among the devices of `kind`; `null` is the default. */
async function deviceIdFor(choice: DeviceChoice, kind: MediaDeviceKind): Promise<string | null> {
  if (choice === DEFAULT_DEVICE) {
    return null;
  }
  const devices = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === kind);
  return resolveDevice(choice, devices);
}

/** Whether this browser can route playback to a chosen speaker. */
export function canChooseOutput(): boolean {
  return typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype;
}

/**
 * One playing consumer: the element that plays it and, when the browser has Web Audio, the
 * gain node that scales it. Web Audio is used because an element's own volume stops at 1 and
 * a quiet person needs more.
 */
/**
 * A shared screen is captured at its own full resolution, up to 4K, and 60 frames a second:
 * the best the screen gives, which the encoder and each viewer's bandwidth then carry as far
 * as they can.
 */
/** A camera is opened at up to 1080p and 30 frames a second, the most most cameras give. */
const CAMERA_QUALITY: MediaTrackConstraints = {
  width: { ideal: 1920 },
  height: { ideal: 1080 },
  frameRate: { ideal: 30 },
};

const SCREEN_QUALITY: MediaTrackConstraints = {
  width: { max: 3840 },
  height: { max: 2160 },
  frameRate: { ideal: 60, max: 60 },
};

interface Player {
  audio: HTMLAudioElement;
  /**
   * The remote stream itself, playing muted. Chromium feeds a remote WebRTC track into Web Audio
   * only while that track also plays through a media element; without one the graph hears
   * silence, while Firefox needs none. Playing it muted costs nothing and is heard nowhere.
   */
  pull: HTMLAudioElement | null;
  source: MediaStreamAudioSourceNode | null;
  gain: GainNode | null;
  destination: MediaStreamAudioDestinationNode | null;
}

export function browserVoiceMedia(): VoiceMedia {
  const players = new Map<string, Player>();
  let context: AudioContext | null = null;
  const audioContext = (): AudioContext | null => {
    if (context === null && typeof AudioContext !== "undefined") {
      context = new AudioContext();
    }
    if (context !== null && context.state === "suspended") {
      void context.resume().catch(() => undefined);
    }
    return context;
  };
  let output: DeviceChoice = DEFAULT_DEVICE;
  const route = async (audio: HTMLAudioElement) => {
    if (!canChooseOutput()) {
      return;
    }
    try {
      await audio.setSinkId((await deviceIdFor(output, "audiooutput")) ?? "");
    } catch {
      // The device is gone or refused; the element keeps playing through the default.
    }
  };
  return {
    createDevice(): Promise<VoiceDevice> {
      // The mediasoup-client types are the same shapes narrowed by `VoiceDevice`.
      return Promise.resolve(new Device() as unknown as VoiceDevice);
    },
    async getMicrophone(choice: DeviceChoice): Promise<MediaStreamTrack> {
      const deviceId = await deviceIdFor(choice, "audioinput");
      const processing = { echoCancellation: true, noiseSuppression: true, autoGainControl: true };
      const open = (constraints: MediaTrackConstraints) =>
        navigator.mediaDevices.getUserMedia({ audio: constraints, video: false });
      let stream: MediaStream;
      try {
        stream = await open(
          deviceId === null ? processing : { ...processing, deviceId: { exact: deviceId } },
        );
      } catch (error) {
        // A remembered microphone that is unplugged is no reason to refuse the call.
        if (
          deviceId !== null &&
          error instanceof DOMException &&
          error.name === "OverconstrainedError"
        ) {
          stream = await open(processing);
        } else {
          throw error;
        }
      }
      const [track] = stream.getAudioTracks();
      if (track === undefined) {
        throw new Error("no microphone");
      }
      return track;
    },
    async setOutput(choice: DeviceChoice): Promise<void> {
      output = choice;
      await Promise.all(Array.from(players.values(), (player) => route(player.audio)));
    },
    async getCamera(choice: DeviceChoice): Promise<MediaStreamTrack> {
      const cameras = (await navigator.mediaDevices.enumerateDevices()).filter(
        (device) => device.kind === "videoinput",
      );
      if (cameras.length === 0) {
        throw new CameraError("none");
      }
      // The chosen camera first, then every other one: a camera another app holds, or one
      // unplugged since it was chosen, is no reason to go without the rest.
      const chosen = choice === DEFAULT_DEVICE ? null : resolveDevice(choice, cameras);
      const attempts: (string | null)[] = [
        chosen,
        ...cameras.map((camera) => camera.deviceId).filter((id) => id !== "" && id !== chosen),
      ];
      let lastError: unknown = null;
      let onlyMissing = true;
      for (const deviceId of attempts) {
        try {
          const stream = await navigator.mediaDevices.getUserMedia({
            audio: false,
            video:
              deviceId === null
                ? CAMERA_QUALITY
                : { ...CAMERA_QUALITY, deviceId: { exact: deviceId } },
          });
          const [track] = stream.getVideoTracks();
          if (track !== undefined) {
            return track;
          }
        } catch (error) {
          if (
            error instanceof DOMException &&
            (error.name === "NotAllowedError" || error.name === "SecurityError")
          ) {
            throw new CameraError("denied", error);
          }
          lastError = error;
          onlyMissing &&= error instanceof DOMException && error.name === "NotFoundError";
        }
      }
      throw new CameraError(onlyMissing ? "none" : "failed", lastError);
    },
    async getScreen(): Promise<ScreenCapture> {
      // Audio is asked for so a shared tab or window can bring its sound; browsers that
      // cannot capture it simply return none.
      const stream = await navigator.mediaDevices.getDisplayMedia({
        video: SCREEN_QUALITY,
        audio: true,
      });
      const [video] = stream.getVideoTracks();
      if (video === undefined) {
        throw new Error("no screen");
      }
      // Some capturers (the desktop shell's among them) hand back a track made without the
      // request's constraints; asked again, it takes them.
      await video.applyConstraints(SCREEN_QUALITY).catch(() => undefined);
      return { video, audio: stream.getAudioTracks()[0] ?? null };
    },
    play(consumerId: string, track: MediaStreamTrack): void {
      const audio = document.createElement("audio");
      audio.autoplay = true;
      audio.dataset.voiceConsumer = consumerId;
      audio.style.display = "none";
      const stream = new MediaStream([track]);
      const player: Player = { audio, pull: null, source: null, gain: null, destination: null };
      const ctx = audioContext();
      if (ctx !== null) {
        const pull = document.createElement("audio");
        pull.muted = true;
        pull.srcObject = stream;
        void pull.play().catch(() => undefined);
        player.pull = pull;
        player.source = ctx.createMediaStreamSource(stream);
        player.gain = ctx.createGain();
        player.destination = ctx.createMediaStreamDestination();
        player.source.connect(player.gain);
        player.gain.connect(player.destination);
        audio.srcObject = player.destination.stream;
      } else {
        audio.srcObject = stream;
      }
      document.body.append(audio);
      players.set(consumerId, player);
      void route(audio)
        .then(() => audio.play())
        .catch(() => undefined);
    },
    stop(consumerId: string): void {
      const player = players.get(consumerId);
      if (player !== undefined) {
        player.source?.disconnect();
        player.gain?.disconnect();
        player.audio.srcObject = null;
        player.audio.remove();
        if (player.pull !== null) {
          player.pull.srcObject = null;
        }
        players.delete(consumerId);
      }
    },
    setVolume(consumerId: string, gain: number): void {
      const player = players.get(consumerId);
      if (player === undefined) {
        return;
      }
      if (player.gain !== null) {
        player.gain.gain.value = gain;
      } else {
        // Without Web Audio the element's own volume is all there is, and it stops at 1.
        player.audio.volume = Math.min(1, gain);
      }
    },
  };
}
