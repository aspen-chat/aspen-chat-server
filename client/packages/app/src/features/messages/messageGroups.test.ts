import { describe, expect, it } from "vitest";
import {
  estimateHeight,
  GROUP_SPAN_MS,
  groupContinuations,
  type GroupCandidate,
  type MessageContents,
} from "@/features/messages/messageGroups";

const noon = new Date(2026, 9, 7, 12, 0).getTime();
const minute = 60 * 1000;

function msg(id: string, author: string, minutes: number, extra: Partial<GroupCandidate> = {}) {
  return {
    id,
    author,
    at: noon + minutes * minute,
    alone: false,
    height: 40,
    breakBefore: false,
    ...extra,
  };
}

describe("groupContinuations", () => {
  it("groups one author's run and starts again with another author", () => {
    const messages = [
      msg("a", "ann", 0),
      msg("b", "ann", 1),
      msg("c", "bob", 2),
      msg("d", "ann", 3),
    ];
    expect([...groupContinuations(messages, 0)]).toEqual(["b"]);
  });

  it("spans at most thirty minutes from the group's first message", () => {
    const messages = [msg("a", "ann", 0), msg("b", "ann", 20), msg("c", "ann", 31)];
    expect([...groupContinuations(messages, 0)]).toEqual(["b"]);
    expect(GROUP_SPAN_MS).toBe(30 * minute);
  });

  it("starts again on a new day", () => {
    const late = new Date(2026, 9, 7, 23, 55).getTime();
    const messages = [
      { ...msg("a", "ann", 0), at: late },
      { ...msg("b", "ann", 0), at: late + 10 * minute },
    ];
    expect(groupContinuations(messages, 0).size).toBe(0);
  });

  it("starts again after a break and around a message that stands alone", () => {
    const messages = [
      msg("a", "ann", 0),
      msg("b", "ann", 1, { breakBefore: true }),
      msg("c", "ann", 2, { alone: true }),
      msg("d", "ann", 3),
      msg("e", "ann", 4),
    ];
    expect([...groupContinuations(messages, 0)]).toEqual(["e"]);
  });

  it("starts again before the group would outgrow its share of the list", () => {
    const messages = ["a", "b", "c", "d"].map((id, i) => msg(id, "ann", i, { height: 100 }));
    // The header's 24px and two messages fit in 250; a third would not.
    expect([...groupContinuations(messages, 250)]).toEqual(["b", "d"]);
  });

  it("keeps what each message was decided to be as older messages arrive above", () => {
    const decided = new Map<string, boolean>();
    const later = ["c", "d", "e"].map((id, i) => msg(id, "ann", 20 + i * 6));
    expect([...groupContinuations(later, 0, decided)]).toEqual(["d", "e"]);
    // Read alone, "c" would now continue "a"'s group, and the thirty minutes would end it at
    // "e"; kept, the groups already drawn stay as they were.
    const all = [msg("a", "ann", 0), msg("b", "ann", 10), ...later];
    expect([...groupContinuations(all, 0, decided)]).toEqual(["b", "d", "e"]);
  });

  it("lets a kept continuation go once the message before it is not its author's", () => {
    const decided = new Map<string, boolean>();
    groupContinuations([msg("a", "ann", 0), msg("b", "ann", 1)], 0, decided);
    expect(groupContinuations([msg("x", "bob", 0), msg("b", "ann", 1)], 0, decided).size).toBe(0);
  });
});

describe("estimateHeight", () => {
  const metrics = { column: 400, lineHeight: 24, charWidth: 8 };
  const empty: MessageContents = {
    content: "",
    pictures: [],
    files: 0,
    cards: 0,
    poll: false,
    reactions: false,
    thread: false,
  };

  it("wraps text at the column's width, line by line", () => {
    // Fifty narrow characters fit a line; a hundred and one take three.
    expect(estimateHeight({ ...empty, content: "x".repeat(101) }, metrics)).toBe(8 + 3 * 24);
    expect(estimateHeight({ ...empty, content: "a\nb" }, metrics)).toBe(8 + 2 * 24);
    // Wide characters take twice the room.
    expect(estimateHeight({ ...empty, content: "字".repeat(26) }, metrics)).toBe(8 + 2 * 24);
  });

  it("draws pictures within the column and their limit, at their proportions", () => {
    const wide = estimateHeight({ ...empty, pictures: [{ width: 800, height: 400 }] }, metrics);
    expect(wide).toBe(8 + 200);
    const tall = estimateHeight({ ...empty, pictures: [{ width: 400, height: 1600 }] }, metrics);
    expect(tall).toBe(8 + 320);
  });
});
