import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { playingSound, soundsEnded } from "../src/soundActivity";

describe("soundsEnded", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("settles at once when nothing plays", async () => {
    let settled = false;
    void soundsEnded(1_000).then(() => (settled = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(settled).toBe(true);
  });

  it("settles once every sound playing has ended, each counted once", async () => {
    const first = playingSound();
    const second = playingSound();
    let settled = false;
    void soundsEnded(10_000).then(() => (settled = true));
    first();
    first();
    await vi.advanceTimersByTimeAsync(0);
    expect(settled).toBe(false);
    second();
    await vi.advanceTimersByTimeAsync(0);
    expect(settled).toBe(true);
  });

  it("settles at its limit while a sound still plays", async () => {
    const ended = playingSound();
    let settled = false;
    void soundsEnded(1_000).then(() => (settled = true));
    await vi.advanceTimersByTimeAsync(999);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(settled).toBe(true);
    ended();
  });
});
