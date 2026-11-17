import { describe, expect, it } from "vitest";
import type { PickedText, Span } from "@/features/messages/picks";
import {
  EMOTICONS,
  convertLast,
  convertTyped,
  emoticonEndingAt,
  emoticonsIn,
  inCode,
} from "./emoticons";

const plain = (text: string): PickedText => ({ text, picks: [] });

/** Types `character` at `at` in `was`, as the box would, and converts what that typed. */
function typeInto(was: string, at: number, character: string, kept: readonly Span[] = []) {
  const text = was.slice(0, at) + character + was.slice(at);
  return convertTyped(plain(text), was, at + character.length, kept);
}

describe("emoticonEndingAt", () => {
  it("finds an emoticon standing on its own", () => {
    expect(emoticonEndingAt(":)", 2)).toEqual({ start: 0, end: 2, emoji: "🙂" });
    expect(emoticonEndingAt("hi :)", 5)).toEqual({ start: 3, end: 5, emoji: "🙂" });
    expect(emoticonEndingAt("hi\n<3", 5)).toEqual({ start: 3, end: 5, emoji: "❤️" });
  });

  it("prefers the longest emoticon", () => {
    expect(emoticonEndingAt("so >:(", 6)).toEqual({ start: 3, end: 6, emoji: "😠" });
    expect(emoticonEndingAt("</3", 3)).toEqual({ start: 0, end: 3, emoji: "💔" });
    expect(emoticonEndingAt(":-)", 3)).toEqual({ start: 0, end: 3, emoji: "🙂" });
  });

  it("finds nothing inside a word", () => {
    expect(emoticonEndingAt("http:/", 6)).toBeNull();
    expect(emoticonEndingAt("a:p", 3)).toBeNull();
    expect(emoticonEndingAt("(:)", 3)).toBeNull();
    expect(emoticonEndingAt("hi", 2)).toBeNull();
  });

  it("leaves out what begins lists", () => {
    expect(emoticonEndingAt("8)", 2)).toBeNull();
    expect(emoticonEndingAt("B)", 2)).toBeNull();
    expect(emoticonEndingAt("D:", 2)).toBeNull();
  });

  it("turns every emoticon into an emoji", () => {
    for (const [emoticon, emoji] of EMOTICONS) {
      expect(emoticonEndingAt(emoticon, emoticon.length)?.emoji).toBe(emoji);
      expect(emoji).not.toBe(emoticon);
    }
  });
});

describe("convertTyped", () => {
  it("converts as the space after an emoticon is typed, caret after the space", () => {
    const conversion = typeInto("hi :)", 5, " ");
    expect(conversion?.held.text).toBe("hi 🙂 ");
    expect(conversion?.caret).toBe("hi 🙂 ".length);
    expect(conversion?.typed.text).toBe("hi :) ");
    expect(conversion?.typedCaret).toBe(6);
    expect(conversion?.emoticon).toEqual({ start: 3, end: 5 });
  });

  it("converts before punctuation and a line's end", () => {
    expect(typeInto("wow :D", 6, "!")?.held.text).toBe("wow 😃!");
    expect(typeInto("(nice :)", 8, ")")?.held.text).toBe("(nice 🙂)");
    expect(typeInto("<3", 2, "\n")?.held.text).toBe("❤️\n");
  });

  it("converts in the middle of the text, keeping what follows", () => {
    const conversion = typeInto("a :) b", 4, ",");
    expect(conversion?.held.text).toBe("a 🙂, b");
    expect(conversion?.caret).toBe("a 🙂,".length);
  });

  it("converts nothing while the emoticon may go on", () => {
    expect(typeInto(":", 1, ")")).toBeNull();
    expect(typeInto(":)", 2, "a")).toBeNull();
    expect(typeInto("x", 1, "D")).toBeNull();
  });

  it("converts nothing but a character typed after the emoticon", () => {
    // Pasted: more than one character arrived.
    expect(convertTyped(plain("hi :) "), "hi", 6, [])).toBeNull();
    // Deleted.
    expect(convertTyped(plain(":) "), ":) x", 3, [])).toBeNull();
    // Typed elsewhere, with the caret not after the emoticon.
    expect(typeInto(":) b", 4, " ")).toBeNull();
  });

  it("leaves what Backspace put back as typed", () => {
    expect(typeInto("hi :)", 5, " ", [{ start: 3, end: 5 }])).toBeNull();
    expect(typeInto("hi :) :)", 8, " ", [{ start: 3, end: 5 }])?.held.text).toBe("hi :) 🙂 ");
  });

  it("leaves code as written", () => {
    expect(typeInto("`a :)`", 5, " ")).toBeNull();
    expect(typeInto("see `a :)", 9, " ")).toBeNull();
    expect(typeInto("``a ` :)", 8, " ")).toBeNull();
    expect(typeInto("```\n:)", 6, "\n")).toBeNull();
    expect(typeInto("`a` :)", 6, " ")?.held.text).toBe("`a` 🙂 ");
  });

  it("leaves a pick's text alone, and keeps the picks around it", () => {
    const held: PickedText = {
      text: "@hi :)",
      picks: [{ kind: "role", id: "r", start: 0, end: 6 }],
    };
    const typed: PickedText = { text: "@hi :) ", picks: held.picks };
    expect(convertTyped(typed, held.text, 7, [])).toBeNull();
    const after: PickedText = {
      text: "@bo :) ",
      picks: [{ kind: "user", id: "u", start: 0, end: 3 }],
    };
    expect(convertTyped(after, "@bo :)", 7, [])?.held).toEqual({
      text: "@bo 🙂 ",
      picks: [{ kind: "user", id: "u", start: 0, end: 3 }],
    });
  });
});

describe("convertLast", () => {
  it("converts an emoticon that ends the text", () => {
    expect(convertLast(plain("hi :)"), []).text).toBe("hi 🙂");
    expect(convertLast(plain("hi xD  \n"), []).text).toBe("hi 😆  \n");
  });

  it("leaves the rest alone", () => {
    expect(convertLast(plain("hi :) there"), []).text).toBe("hi :) there");
    expect(convertLast(plain("a:)"), []).text).toBe("a:)");
    expect(convertLast(plain("`a :)"), []).text).toBe("`a :)");
    expect(convertLast(plain("hi :)"), [{ start: 3, end: 5 }]).text).toBe("hi :)");
  });
});

describe("emoticonsIn", () => {
  it("finds each emoticon standing on its own", () => {
    expect(emoticonsIn("a :) b xD, c:) <3")).toEqual([
      { start: 2, end: 4, emoji: "🙂" },
      { start: 7, end: 9, emoji: "😆" },
      { start: 15, end: 17, emoji: "❤️" },
    ]);
    expect(emoticonsIn(":)a")).toEqual([]);
  });
});

describe("inCode", () => {
  it("reads code as Markdown does", () => {
    expect(inCode("no code :)", { start: 8, end: 10 })).toBe(false);
    expect(inCode("~~~\n:)", { start: 4, end: 6 })).toBe(true);
    expect(inCode("`a` and :)", { start: 8, end: 10 })).toBe(false);
  });
});
