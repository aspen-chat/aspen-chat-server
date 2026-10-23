import { describe, expect, it } from "vitest";
import { formatClock } from "./clock";

describe("formatClock", () => {
  it("writes minutes and seconds under an hour, and hours from there", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(65.9)).toBe("1:05");
    expect(formatClock(3600)).toBe("1:00:00");
    expect(formatClock(3725)).toBe("1:02:05");
    expect(formatClock(-3)).toBe("0:00");
  });

  it("takes as many places as the longest time it is shown beside", () => {
    expect(formatClock(303, 3720)).toBe("0:05:03");
    expect(formatClock(303, 600)).toBe("5:03");
  });
});
