/**
 * The browser's side of a voice call: the microphone through `getUserMedia`, mediasoup-client's
 * `Device` for the WebRTC transports, and playback through hidden `<audio>` elements, one per
 * consumer, appended to the document so autoplay policy sees a playing element per track.
 */

import { Device } from "mediasoup-client";
import type { ScreenCapture, VoiceDevice, VoiceMedia } from "./voice";

export function browserVoiceMedia(): VoiceMedia {
  const players = new Map<string, HTMLAudioElement>();
  return {
    createDevice(): Promise<VoiceDevice> {
      // The mediasoup-client types are the same shapes narrowed by `VoiceDevice`.
      return Promise.resolve(new Device() as unknown as VoiceDevice);
    },
    async getMicrophone(): Promise<MediaStreamTrack> {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
        video: false,
      });
      const [track] = stream.getAudioTracks();
      if (track === undefined) {
        throw new Error("no microphone");
      }
      return track;
    },
    async getScreen(): Promise<ScreenCapture> {
      // Audio is asked for so a shared tab or window can bring its sound; browsers that
      // cannot capture it simply return none.
      const stream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
      const [video] = stream.getVideoTracks();
      if (video === undefined) {
        throw new Error("no screen");
      }
      return { video, audio: stream.getAudioTracks()[0] ?? null };
    },
    play(consumerId: string, track: MediaStreamTrack): void {
      const audio = document.createElement("audio");
      audio.autoplay = true;
      audio.dataset.voiceConsumer = consumerId;
      audio.srcObject = new MediaStream([track]);
      audio.style.display = "none";
      document.body.append(audio);
      players.set(consumerId, audio);
      void audio.play().catch(() => undefined);
    },
    stop(consumerId: string): void {
      const audio = players.get(consumerId);
      if (audio !== undefined) {
        audio.srcObject = null;
        audio.remove();
        players.delete(consumerId);
      }
    },
  };
}
