import type { CustomEmoji } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { decodeCustomEmoji, emojiIdOf, encodeCustomEmoji, referenceOf } from "./customEmoji";
import { emojiLanguageFor } from "./emojiData";
import { searchEmojiNames } from "./emojiNames";
import { emojiQueryAt } from "./useEmojiCompletion";

const parrot: CustomEmoji = {
  id: "0190f0a0-0000-7000-8000-0000000000e1",
  community: "c",
  name: "PartyParrot",
  icon: "i",
  createdBy: null,
};
const blob: CustomEmoji = { ...parrot, id: "0190f0a0-0000-7000-8000-0000000000e2", name: "blob" };

describe("custom emoji references", () => {
  it("writes and reads a reference", () => {
    expect(referenceOf(parrot.id)).toBe("<:0190f0a0-0000-7000-8000-0000000000e1>");
    expect(emojiIdOf("<:0190F0A0-0000-7000-8000-0000000000E1>")).toBe(parrot.id);
    expect(emojiIdOf("👍")).toBeNull();
    expect(emojiIdOf("<:nope>")).toBeNull();
  });

  it("encodes :name: ignoring case, and leaves unknown names and code-like colons alone", () => {
    expect(encodeCustomEmoji("hi :partyparrot: and :BLOB:!", [parrot, blob])).toBe(
      `hi ${referenceOf(parrot.id)} and ${referenceOf(blob.id)}!`,
    );
    expect(encodeCustomEmoji("at 10:30:45 :nope:", [parrot])).toBe("at 10:30:45 :nope:");
    expect(encodeCustomEmoji(":partyparrot:", [])).toBe(":partyparrot:");
  });

  it("decodes references to :name:, leaving unknown ones as sent", () => {
    const sent = `${referenceOf(parrot.id)} ${referenceOf("0190f0a0-0000-7000-8000-0000000000e9")}`;
    expect(decodeCustomEmoji(sent, [parrot])).toBe(
      ":PartyParrot: <:0190f0a0-0000-7000-8000-0000000000e9>",
    );
  });
});

describe("emojiQueryAt", () => {
  it("finds a colon word being typed, not a colon inside a word", () => {
    expect(emojiQueryAt("hello :par", 10)).toEqual({ start: 6, query: "par" });
    expect(emojiQueryAt(":pa", 3)).toEqual({ start: 0, query: "pa" });
    expect(emojiQueryAt("see http://x", 12)).toBeNull();
    expect(emojiQueryAt("at 10:30", 8)).toBeNull();
    expect(emojiQueryAt("done :tada: now", 15)).toBeNull();
  });
});

describe("searchEmojiNames", () => {
  const named = [
    { glyph: "🎉", names: ["party", "tada", "party popper"] },
    { glyph: "🦜", names: ["bird", "parrot"] },
    { glyph: "🍰", names: ["cake", "dessert", "shortcake"] },
  ];
  it("puts names that begin with the query first, then names that hold it", () => {
    expect(searchEmojiNames(named, "par", 5).map((e) => e.glyph)).toEqual(["🎉", "🦜"]);
    expect(searchEmojiNames(named, "cake", 5).map((e) => e.glyph)).toEqual(["🍰"]);
    expect(searchEmojiNames(named, "ort", 5).map((e) => e.glyph)).toEqual(["🍰"]);
    expect(searchEmojiNames(named, "zzz", 5)).toEqual([]);
  });
});

describe("emojiLanguageFor", () => {
  it("follows the browser's languages when the app's language is automatic", () => {
    expect(emojiLanguageFor("automatic", ["fr-CA", "en-US"])).toBe("fr");
    expect(emojiLanguageFor("automatic", ["zh-Hant-TW", "zh"])).toBe("zh-hant");
    expect(emojiLanguageFor("automatic", ["en-GB", "en"])).toBe("en-gb");
    expect(emojiLanguageFor("automatic", ["tlh", "eo"])).toBe("en");
    expect(emojiLanguageFor("automatic", [])).toBe("en");
  });

  it("follows a chosen catalogue, pseudo-locales being English", () => {
    expect(emojiLanguageFor("en", ["fr"])).toBe("en");
    expect(emojiLanguageFor("en-XA", ["fr"])).toBe("en");
    expect(emojiLanguageFor("ar-XB", ["fr"])).toBe("en");
  });
});
