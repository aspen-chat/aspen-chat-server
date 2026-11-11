import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { axisDate, describer } from "@/features/admin/growthDates";
import { en } from "@/i18n/messages";

describe("a growth point's name", () => {
  let zone: string | undefined;
  beforeEach(() => {
    zone = process.env.TZ;
  });
  afterEach(() => {
    if (zone === undefined) {
      delete process.env.TZ;
    } else {
      process.env.TZ = zone;
    }
  });

  it.each(["America/Los_Angeles", "UTC", "Pacific/Auckland"])(
    "is its step of UTC's calendar for a viewer in %s",
    (viewer) => {
      process.env.TZ = viewer;
      // A Sunday's midnight, the first of a month, in UTC.
      const at = "2026-11-01T00:00:00Z";
      expect(describer(en, "en-US", "day")(at)).toBe("Nov 1");
      expect(describer(en, "en-US", "week")(at)).toContain("Nov 1");
      expect(describer(en, "en-US", "month")(at)).toBe("Nov 2026");
      expect(axisDate("en-US", "day")(at)).toBe("Nov 1");
      expect(axisDate("en-US", "month")(at)).toBe("Nov 26");
    },
  );
});
