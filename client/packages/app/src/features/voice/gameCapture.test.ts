import { describe, expect, it } from "vitest";
import {
  applicationAudioShare,
  captureChoice,
  gameCaptureShare,
  testPattern,
  type GameCaptureBridge,
} from "./gameCapture";

const target = {
  ip: "192.0.2.10",
  port: 40000,
  ssrc: 7,
  payloadType: 96,
  srtpCryptoSuite: "AES_CM_128_HMAC_SHA1_80",
  srtpKeyBase64: "a2V5",
};

/** A bridge that records what it is asked to do and lets a test end the capture. */
function recordingBridge(fail?: Error) {
  const calls: string[] = [];
  const ended: { listener: (() => void) | null } = { listener: null };
  const bridge: GameCaptureBridge = {
    kinds: () => Promise.resolve({ kinds: [], applicationAudio: null, testMedia: null }),
    start: (options) => {
      const audio =
        options.audio === undefined
          ? "none"
          : `${options.audio.kind}@${String(options.audio.rtp.port)}`;
      calls.push(`start ${options.kind} ${String(options.rtp.port)} audio=${audio}`);
      return fail === undefined ? Promise.resolve() : Promise.reject(fail);
    },
    startAudio: (audio) => {
      calls.push(`startAudio ${audio.kind} ${audio.settings ?? ""} ${String(audio.rtp.port)}`);
      return fail === undefined ? Promise.resolve() : Promise.reject(fail);
    },
    stop: () => {
      calls.push("stop");
      return Promise.resolve();
    },
    onEnded: (listener) => {
      ended.listener = listener;
      return () => {
        ended.listener = null;
      };
    },
  };
  return { bridge, calls, ended };
}

describe("captureChoice", () => {
  it("speaks each source's vocabulary and points the audio source at the same window", () => {
    const game = {
      kind: "game_capture",
      property: "window",
      targets: [],
      audio: { kind: "wasapi_process_output_capture", property: "window", targets: null },
    };
    expect(captureChoice(game, { name: "Doom", value: "Doom:Class:doom.exe" })).toEqual({
      kind: "game_capture",
      settings: { capture_mode: "window", window: "Doom:Class:doom.exe" },
      audio: {
        kind: "wasapi_process_output_capture",
        settings: { window: "Doom:Class:doom.exe" },
      },
    });
    const mac = {
      kind: "screen_capture",
      property: "application",
      targets: [],
      audio: { kind: "sck_audio_capture", property: "application", targets: null },
    };
    expect(captureChoice(mac, { name: "Doom", value: "com.id.doom" })).toEqual({
      kind: "screen_capture",
      settings: { type: 2, application: "com.id.doom" },
      audio: { kind: "sck_audio_capture", settings: { application: "com.id.doom" } },
    });
    const silent = { kind: "window_capture", property: "window", targets: [], audio: null };
    expect(captureChoice(silent, { name: "Doom", value: "w" }).audio).toBeNull();
  });

  it("offers a silent colour source without a clip, and the clip with its sound with one", () => {
    expect(testPattern(null).audio).toBeNull();
    expect(testPattern("/tmp/clip.mp4")).toEqual({
      kind: "ffmpeg_source",
      settings: {
        is_local_file: true,
        local_file: "/tmp/clip.mp4",
        looping: true,
        hw_decode: false,
      },
      audio: { kind: "", settings: {} },
    });
  });
});

describe("gameCaptureShare", () => {
  const targets = { video: target, audio: { ...target, port: 40001, payloadType: 100 } };

  it("starts the helper at the producers' targets and stops it, watching for its death meanwhile", async () => {
    const { bridge, calls, ended } = recordingBridge();
    let endings = 0;
    const share = gameCaptureShare(
      bridge,
      { kind: "color_source", settings: {}, audio: { kind: "", settings: {} } },
      true,
      () => {
        endings += 1;
      },
    );
    expect(share.audio).toBe(true);
    await share.start(targets);
    expect(calls).toEqual(["start color_source 40000 audio=@40001"]);
    ended.listener?.();
    expect(endings).toBe(1);
    share.stop();
    expect(calls).toEqual(["start color_source 40000 audio=@40001", "stop"]);
    expect(ended.listener).toBeNull();
  });

  it("leaves audio out when the user declines it, and stops watching when the helper refuses", async () => {
    const { bridge, calls, ended } = recordingBridge(new Error("no such source"));
    const declined = gameCaptureShare(
      bridge,
      { kind: "nope", settings: {}, audio: { kind: "x", settings: {} } },
      false,
      () => undefined,
    );
    expect(declined.audio).toBe(false);
    await expect(declined.start({ video: target, audio: null })).rejects.toThrow("no such source");
    expect(calls).toEqual(["start nope 40000 audio=none"]);
    expect(ended.listener).toBeNull();
  });
});

describe("applicationAudioShare", () => {
  const kind = { kind: "aspen_pipewire_app_audio", property: "", targets: [] };
  const doom = { name: "doom.exe", pid: 20, settings: { application: "doom.exe", pid: 20 } };

  it("captures the chosen application's sound alone at the producer's target", async () => {
    const { bridge, calls, ended } = recordingBridge();
    let endings = 0;
    const audio = applicationAudioShare(bridge, kind, doom, () => {
      endings += 1;
    });
    await audio.start({ ...target, port: 40001, payloadType: 100 });
    expect(calls).toEqual([
      'startAudio aspen_pipewire_app_audio {"application":"doom.exe","pid":20} 40001',
    ]);
    ended.listener?.();
    expect(endings).toBe(1);
    audio.stop();
    expect(calls.at(-1)).toBe("stop");
    expect(ended.listener).toBeNull();
  });

  it("stops watching the helper when it refuses", async () => {
    const { bridge, ended } = recordingBridge(new Error("PipeWire did not answer"));
    const audio = applicationAudioShare(bridge, kind, doom, () => undefined);
    await expect(audio.start(target)).rejects.toThrow("PipeWire did not answer");
    expect(ended.listener).toBeNull();
  });
});
