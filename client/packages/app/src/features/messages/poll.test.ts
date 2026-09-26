import type { Poll } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { optionName, pollOpen, pollOutcome, sharePercent, timeLeft } from "./poll";

function poll(counts: number[], extra: Partial<Poll> = {}): Poll {
  return {
    id: "p",
    channelId: "c",
    messageId: "m",
    createdBy: "u",
    createdAt: "2026-09-25T12:00:00Z",
    closesAt: "2026-09-25T13:00:00Z",
    closedAt: null,
    question: "q",
    options: counts.map((_, i) => ({ label: `o${String(i)}` })),
    multipleChoice: false,
    anonymous: true,
    results: counts.map((count) => ({ count })),
    ...extra,
  };
}

describe("polls", () => {
  it("reports a winner, a tie, or no votes", () => {
    expect(pollOutcome(poll([0, 0]))).toEqual({ kind: "noVotes" });
    expect(pollOutcome(poll([1, 3, 2]))).toEqual({ kind: "winner", option: 1, count: 3 });
    expect(pollOutcome(poll([2, 2, 1]))).toEqual({ kind: "tie", options: [0, 1], count: 2 });
  });

  it("is open until the deadline or an explicit close", () => {
    const before = Date.parse("2026-09-25T12:59:59Z");
    expect(pollOpen(poll([]), before)).toBe(true);
    expect(pollOpen(poll([]), Date.parse("2026-09-25T13:00:00Z"))).toBe(false);
    expect(pollOpen(poll([], { closedAt: "2026-09-25T12:30:00Z" }), before)).toBe(false);
  });

  it("breaks the time left into units and never goes negative", () => {
    const closesAt = "2026-09-27T14:30:05Z";
    expect(timeLeft(closesAt, Date.parse("2026-09-25T12:00:00Z"))).toEqual({
      days: 2,
      hours: 2,
      minutes: 30,
      seconds: 5,
    });
    expect(timeLeft(closesAt, Date.parse("2026-09-28T00:00:00Z"))).toEqual({
      days: 0,
      hours: 0,
      minutes: 0,
      seconds: 0,
    });
  });

  it("names an option by its emoji and label", () => {
    expect(optionName({ label: "Pizza", emoji: "🍕" })).toBe("🍕 Pizza");
    expect(optionName({ label: "Sushi" })).toBe("Sushi");
    expect(optionName(undefined)).toBe("");
  });

  it("gives each option its share of the votes cast", () => {
    expect(sharePercent(poll([1, 3]), 1)).toBe(75);
    expect(sharePercent(poll([0, 0]), 0)).toBe(0);
  });
});
