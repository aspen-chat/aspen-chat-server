import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { browserVoiceMedia, MICROPHONE_HOLD_LIMIT_MS } from "../src/browserMedia";
import { playingSound } from "../src/soundActivity";

/** Stands in for `AudioContext`, with just the nodes the call's media makes. */
class FakeContext {
  static made: FakeContext[] = [];
  state: AudioContextState = "running";
  constructor() {
    FakeContext.made.push(this);
  }
  close(): Promise<void> {
    this.state = "closed";
    return Promise.resolve();
  }
  createMediaStreamSource() {
    return { connect: () => undefined, disconnect: () => undefined };
  }
  createGain() {
    return { gain: { value: 1 }, connect: () => undefined, disconnect: () => undefined };
  }
  createMediaStreamDestination() {
    return { stream: { getAudioTracks: () => [track()] } };
  }
}

function track(): MediaStreamTrack {
  return { stop: () => undefined } as unknown as MediaStreamTrack;
}

function element() {
  return {
    dataset: {},
    style: {},
    play: () => Promise.resolve(),
    remove: () => undefined,
  };
}

describe("browserVoiceMedia", () => {
  beforeEach(() => {
    FakeContext.made = [];
    vi.stubGlobal("AudioContext", FakeContext);
    vi.stubGlobal(
      "MediaStream",
      class {
        constructor(readonly tracks: MediaStreamTrack[]) {}
      },
    );
    vi.stubGlobal("document", { createElement: element, body: { append: () => undefined } });
    vi.stubGlobal("navigator", {
      mediaDevices: {
        getUserMedia: () => Promise.resolve({ getAudioTracks: () => [track()] }),
      },
    });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("closes the call's context once nothing uses it, and opens another for the next call", async () => {
    const media = browserVoiceMedia();
    const microphone = await media.getMicrophone("default", 1);
    media.play("voice", track());
    expect(FakeContext.made).toHaveLength(1);
    microphone.stop();
    expect(FakeContext.made[0]?.state).toBe("running");
    media.stop("voice");
    expect(FakeContext.made[0]?.state).toBe("closed");
    await media.getMicrophone("default", 1);
    expect(FakeContext.made).toHaveLength(2);
    expect(FakeContext.made[1]?.state).toBe("running");
  });

  describe("releasing the microphone", () => {
    let captured: { stop: ReturnType<typeof vi.fn> };
    beforeEach(() => {
      vi.useFakeTimers();
      captured = { stop: vi.fn() };
      vi.stubGlobal("navigator", {
        mediaDevices: {
          getUserMedia: () => Promise.resolve({ getAudioTracks: () => [captured] }),
        },
      });
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    it("waits for a sound asked for in the same task as the stop to end", async () => {
      const microphone = await browserVoiceMedia().getMicrophone("default", 1);
      microphone.stop();
      const ended = playingSound();
      await vi.advanceTimersByTimeAsync(1_500);
      expect(captured.stop).not.toHaveBeenCalled();
      ended();
      await vi.advanceTimersByTimeAsync(0);
      expect(captured.stop).toHaveBeenCalledOnce();
    });

    it("is released in the next task when no sound plays", async () => {
      const microphone = await browserVoiceMedia().getMicrophone("default", 1);
      microphone.stop();
      expect(captured.stop).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(0);
      expect(captured.stop).toHaveBeenCalledOnce();
    });

    it("is released after MICROPHONE_HOLD_LIMIT_MS however long a sound goes on", async () => {
      const microphone = await browserVoiceMedia().getMicrophone("default", 1);
      const ended = playingSound();
      microphone.stop();
      await vi.advanceTimersByTimeAsync(MICROPHONE_HOLD_LIMIT_MS - 1);
      expect(captured.stop).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(captured.stop).toHaveBeenCalledOnce();
      ended();
    });
  });
});
