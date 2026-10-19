import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** Stands in for `HTMLAudioElement`, recording what is playing and letting a test end it. */
class FakeAudio extends EventTarget {
  static playing = new Set<FakeAudio>();
  loop = false;
  constructor(readonly src: string) {
    super();
  }
  play(): Promise<void> {
    FakeAudio.playing.add(this);
    return Promise.resolve();
  }
  pause(): void {
    FakeAudio.playing.delete(this);
  }
  end(): void {
    FakeAudio.playing.delete(this);
    this.dispatchEvent(new Event("ended"));
  }
}

/** A fresh copy of the module, so its counts do not carry between tests. */
async function sounds() {
  vi.resetModules();
  return import("./sounds");
}

describe("sound limits", () => {
  beforeEach(() => {
    FakeAudio.playing.clear();
    vi.stubGlobal("Audio", FakeAudio);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("plays at most two of one sound at once", async () => {
    const { playSound, MAX_PLAYING_PER_SOUND } = await sounds();
    for (let i = 0; i < 5; i++) {
      await playSound("chime", null);
    }
    expect(FakeAudio.playing.size).toBe(MAX_PLAYING_PER_SOUND);
  });

  it("counts each sound on its own", async () => {
    const { playSound, loopSound } = await sounds();
    await playSound("chime", null);
    await playSound("chime", null);
    loopSound("ringtone", null);
    await vi.waitFor(() => {
      expect(FakeAudio.playing.size).toBe(3);
    });
  });

  it("plays again once one has ended", async () => {
    const { playSound } = await sounds();
    await playSound("chime", null);
    await playSound("chime", null);
    const [first] = FakeAudio.playing;
    if (first === undefined) {
      throw new Error("Nothing is playing");
    }
    first.end();
    await playSound("chime", null);
    expect(FakeAudio.playing.size).toBe(2);
    expect(FakeAudio.playing.has(first)).toBe(false);
  });

  it("gives a loop's place back when it is stopped", async () => {
    const { loopSound } = await sounds();
    const stopFirst = loopSound("ringtone", null);
    loopSound("ringtone", null);
    const refused = loopSound("ringtone", null);
    await vi.waitFor(() => {
      expect(FakeAudio.playing.size).toBe(2);
    });
    refused();
    stopFirst();
    stopFirst();
    expect(FakeAudio.playing.size).toBe(1);
    loopSound("ringtone", null);
    loopSound("ringtone", null);
    await vi.waitFor(() => {
      expect(FakeAudio.playing.size).toBe(2);
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(FakeAudio.playing.size).toBe(2);
  });

  it("gives the place back when the browser refuses to play", async () => {
    const { playSound } = await sounds();
    const play = vi
      .spyOn(FakeAudio.prototype, "play")
      .mockRejectedValue(new DOMException("", "NotAllowedError"));
    await playSound("chime", null);
    await playSound("chime", null);
    play.mockRestore();
    await playSound("chime", null);
    expect(FakeAudio.playing.size).toBe(1);
  });
});
