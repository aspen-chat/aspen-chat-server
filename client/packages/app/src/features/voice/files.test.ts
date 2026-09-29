import { describe, expect, it } from "vitest";
import type { OfferState } from "@aspen/protocol";
import { formatSize, formatTimeLeft, linkPath, receiveModes } from "./files";

const offer = (allowDirect: boolean): OfferState => ({
  id: "o",
  from: "u",
  name: "f",
  size: 1,
  allowDirect,
  expiresAt: 0,
  own: false,
});

describe("files", () => {
  it("formats sizes in the largest unit that fits", () => {
    expect(formatSize(512, "en")).toBe("512B");
    expect(formatSize(1_500, "en")).toBe("1.5 kB");
    expect(formatSize(2_000_000_000, "en")).toBe("2 GB");
  });

  it("counts time left down to the second", () => {
    expect(formatTimeLeft(59_001)).toBe("1:00");
    expect(formatTimeLeft(5_000)).toBe("0:05");
    expect(formatTimeLeft(3_600_000)).toBe("1:00:00");
    expect(formatTimeLeft(-1)).toBe("0:00");
  });

  it("offers only the ways the sender and the server allow", () => {
    expect(receiveModes(offer(true), 50)).toEqual(["directPreferred", "relayOnly"]);
    expect(receiveModes(offer(false), 50)).toEqual(["relayOnly"]);
    expect(receiveModes(offer(true), null)).toEqual(["directPreferred"]);
  });

  it("routes links with right angles only", () => {
    const tile = (left: number, top: number) => ({ left, top, width: 100, height: 80 });
    // Same row: down, across, and back up below the row.
    expect(linkPath(tile(0, 0), tile(200, 0), 12)).toBe("M50,80 V92 H250 V80");
    // Receiver in a lower row: down into the gap, across, and down into it.
    expect(linkPath(tile(0, 0), tile(200, 92), 12)).toBe("M50,80 V86 H250 V92");
    // Receiver above: out of the top.
    expect(linkPath(tile(200, 92), tile(0, 0), 12)).toBe("M250,92 V86 H50 V80");
    for (const path of [linkPath(tile(0, 0), tile(200, 92), 12, 2)]) {
      expect(path).toMatch(/^M[\d.]+,[\d.]+ V[\d.]+ H[\d.]+ V[\d.]+$/);
    }
  });
});
