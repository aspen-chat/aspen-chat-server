import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Playing } from "./speakers";

const { play } = vi.hoisted(() => ({ play: vi.fn<() => Promise<Playing>>() }));

vi.mock("./speakers", () => ({ play }));

/** Stands in for a sound `play` started, recording what is playing and letting a test end it. */
class FakePlaying implements Playing {
  static playing = new Set<FakePlaying>();
  ended: Promise<void>;
  private resolve: () => void = () => undefined;
  constructor() {
    this.ended = new Promise((resolve) => {
      this.resolve = resolve;
    });
    FakePlaying.playing.add(this);
  }
  stop(): void {
    this.end();
  }
  end(): void {
    FakePlaying.playing.delete(this);
    this.resolve();
  }
}

/** A fresh copy of the module, so its counts do not carry between tests. */
async function sounds() {
  vi.resetModules();
  return import("./sounds");
}

describe("sound limits", () => {
  beforeEach(() => {
    play.mockImplementation(() => Promise.resolve(new FakePlaying()));
  });
  afterEach(() => {
    FakePlaying.playing.clear();
    play.mockReset();
  });

  it("plays at most two of one sound at once", async () => {
    const { playSound, MAX_PLAYING_PER_SOUND } = await sounds();
    for (let i = 0; i < 5; i++) {
      await playSound("chime", null);
    }
    expect(FakePlaying.playing.size).toBe(MAX_PLAYING_PER_SOUND);
  });

  it("counts each sound on its own", async () => {
    const { playSound, loopSound } = await sounds();
    await playSound("chime", null);
    await playSound("chime", null);
    loopSound("ringtone", null);
    await vi.waitFor(() => {
      expect(FakePlaying.playing.size).toBe(3);
    });
  });

  it("plays again once one has ended", async () => {
    const { playSound } = await sounds();
    await playSound("chime", null);
    await playSound("chime", null);
    const [first] = FakePlaying.playing;
    if (first === undefined) {
      throw new Error("Nothing is playing");
    }
    first.end();
    await first.ended;
    await playSound("chime", null);
    expect(FakePlaying.playing.size).toBe(2);
    expect(FakePlaying.playing.has(first)).toBe(false);
  });

  it("gives a loop's place back when it is stopped", async () => {
    const { loopSound } = await sounds();
    const stopFirst = loopSound("ringtone", null);
    loopSound("ringtone", null);
    const refused = loopSound("ringtone", null);
    await vi.waitFor(() => {
      expect(FakePlaying.playing.size).toBe(2);
    });
    refused();
    stopFirst();
    stopFirst();
    expect(FakePlaying.playing.size).toBe(1);
    loopSound("ringtone", null);
    loopSound("ringtone", null);
    await vi.waitFor(() => {
      expect(FakePlaying.playing.size).toBe(2);
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(FakePlaying.playing.size).toBe(2);
  });

  it("stops a loop stopped before it started", async () => {
    const { loopSound } = await sounds();
    loopSound("ringtone", null)();
    await vi.waitFor(() => {
      expect(play).toHaveBeenCalledOnce();
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(FakePlaying.playing.size).toBe(0);
  });

  it("gives the place back when the browser refuses to play", async () => {
    const { playSound } = await sounds();
    play.mockRejectedValueOnce(new DOMException("", "NotAllowedError"));
    play.mockRejectedValueOnce(new DOMException("", "NotAllowedError"));
    await playSound("chime", null);
    await playSound("chime", null);
    await playSound("chime", null);
    expect(FakePlaying.playing.size).toBe(1);
  });
});
