import { ServerClock, type VoiceRing } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { ringing } from "./ringing";

const T = Date.parse("2026-11-11T12:00:00Z");
/** A ring the server made at `T`, for its usual thirty seconds. */
const ring: VoiceRing = {
  session: "s",
  channel: "c",
  user: "u",
  caller: "b",
  until: new Date(T + 30_000).toISOString(),
};

/** The clock of a server at `T` as a device at `device` hears it. */
function heard(device: number): ServerClock {
  const clock = new ServerClock();
  clock.note(new Date(T).toISOString(), device);
  return clock;
}

describe("ringing", () => {
  it("rings for the ring's whole life on a device whose clock is fast", () => {
    const device = T + 40_000;
    const clock = heard(device);
    expect(ringing(ring, clock, device)).toBe(true);
    expect(ringing(ring, clock, device + 29_000)).toBe(true);
    expect(ringing(ring, clock, device + 30_000)).toBe(false);
  });

  it("stops with every other device on one whose clock is slow", () => {
    const device = T - 40_000;
    const clock = heard(device);
    expect(ringing(ring, clock, device + 29_000)).toBe(true);
    expect(ringing(ring, clock, device + 31_000)).toBe(false);
  });

  it("goes by the device's clock for a server that gives no time", () => {
    const clock = new ServerClock();
    clock.note(null, T);
    expect(ringing(ring, clock, T + 29_000)).toBe(true);
    expect(ringing(ring, clock, T + 31_000)).toBe(false);
  });
});
