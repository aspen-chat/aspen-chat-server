import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { browserVoiceMedia } from "../src/browserMedia";

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
});
