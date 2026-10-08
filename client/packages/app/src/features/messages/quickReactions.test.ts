import { describe, expect, it } from "vitest";
import { referenceOf } from "@/features/emoji/customEmoji";
import { DEFAULT_QUICK_REACTIONS, quickReactions } from "@/features/messages/quickReactions";

const parrot = "0190f0a0-0000-7000-8000-0000000000e1";
const gone = "0190f0a0-0000-7000-8000-0000000000e2";

describe("quickReactions", () => {
  it("offers the defaults to someone who has not reacted", () => {
    expect(quickReactions([], new Set())).toEqual(DEFAULT_QUICK_REACTIONS);
  });

  it("takes the most used first and fills the rest with defaults not already offered", () => {
    expect(quickReactions(["\u{1F389}", "\u{1F44D}"], new Set())).toEqual([
      "\u{1F389}",
      "\u{1F44D}",
      "\u{1F604}",
      "\u{2764}\u{FE0F}",
      "\u{1F44E}",
    ]);
  });

  it("offers five at most", () => {
    const used = ["1", "2", "3", "4", "5", "6"].map((n) => `${n}\u{FE0F}\u{20E3}`);
    expect(quickReactions(used, new Set())).toEqual(used.slice(0, 5));
  });

  it("skips custom emoji that cannot be used here", () => {
    expect(quickReactions([referenceOf(gone), referenceOf(parrot)], new Set([parrot]))).toEqual([
      referenceOf(parrot),
      ...DEFAULT_QUICK_REACTIONS.slice(0, 4),
    ]);
  });
});
