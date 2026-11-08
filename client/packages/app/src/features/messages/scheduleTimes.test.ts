import { describe, expect, it } from "vitest";
import {
  localDateTime,
  MAX_AHEAD_MS,
  quickTimes,
  timeProblem,
  toDate,
} from "@/features/messages/scheduleTimes";

describe("quickTimes", () => {
  it("offers an hour on, tomorrow morning, and the coming Monday's", () => {
    // A Thursday afternoon.
    const now = new Date(2026, 10, 5, 15, 38, 42);
    const times = quickTimes(now);
    expect(times.map((time) => time.key)).toEqual(["inAnHour", "tomorrow", "nextMonday"]);
    expect(times[0]?.at).toEqual(new Date(2026, 10, 5, 16, 38, 0));
    expect(times[1]?.at).toEqual(new Date(2026, 10, 6, 9, 0, 0));
    expect(times[2]?.at).toEqual(new Date(2026, 10, 9, 9, 0, 0));
  });

  it("leaves Monday out on a Sunday, when tomorrow is that Monday", () => {
    const sunday = new Date(2026, 10, 8, 10, 0, 0);
    expect(quickTimes(sunday).map((time) => time.key)).toEqual(["inAnHour", "tomorrow"]);
  });

  it("means the Monday a week on when today is a Monday", () => {
    const monday = new Date(2026, 10, 9, 8, 0, 0);
    expect(quickTimes(monday).at(-1)).toEqual({
      key: "nextMonday",
      at: new Date(2026, 10, 16, 9, 0, 0),
    });
  });
});

describe("the date field's values", () => {
  it("are the reader's own date and time, and name the same moment back", () => {
    const at = new Date(2027, 0, 3, 7, 5, 0);
    const held = localDateTime(at);
    expect([held.year, held.month, held.day, held.hour, held.minute]).toEqual([2027, 1, 3, 7, 5]);
    expect(toDate(held)).toEqual(at);
  });
});

describe("timeProblem", () => {
  const now = new Date(2026, 10, 5, 12, 0, 0);

  it("refuses under a minute ahead and over two years", () => {
    expect(timeProblem(new Date(now.getTime() + 30_000), now)).toBe("tooSoon");
    expect(timeProblem(new Date(now.getTime() - 1), now)).toBe("tooSoon");
    expect(timeProblem(new Date(now.getTime() + MAX_AHEAD_MS + 1), now)).toBe("tooFar");
  });

  it("takes anything between", () => {
    expect(timeProblem(new Date(now.getTime() + 60_000), now)).toBeNull();
    expect(timeProblem(new Date(now.getTime() + MAX_AHEAD_MS), now)).toBeNull();
  });
});
