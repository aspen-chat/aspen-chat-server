import { describe, expect, it } from "vitest";
import { timeZones } from "./timeZones";

describe("timeZones", () => {
  it("lists the browser's zones in order, with UTC and the one chosen", () => {
    const zones = timeZones("Mars/Olympus_Mons");
    expect(zones).toContain("UTC");
    expect(zones).toContain("Europe/Berlin");
    expect(zones).toContain("Mars/Olympus_Mons");
    expect(zones).toEqual([...zones].sort());
    expect(new Set(zones).size).toBe(zones.length);
  });
});
