import { describe, expect, it } from "vitest";
import { REPLY_PREVIEW_MAX_CHARS, replyPreview as preview } from "./replyPreview";

const replyPreview = (content: string, spoiler: string) =>
  preview(content, spoiler, (text) => text);

describe("replyPreview", () => {
  it("keeps one line of the text", () => {
    expect(replyPreview("first line\n\n  second\tline ", "[spoiler]")).toBe(
      "first line second line",
    );
  });

  it("never shows what a spoiler hides, in either spelling or across lines", () => {
    expect(replyPreview("it was ||the butler|| and >!the cook!<", "[spoiler]")).toBe(
      "it was [spoiler] and [spoiler]",
    );
    expect(replyPreview("||one\ntwo||", "[spoiler]")).toBe("[spoiler]");
  });

  it("hides what the message hides and nothing else", () => {
    // A spoiler holds markup and code, and a marker in code is none.
    expect(replyPreview("a || `code` **b** || c and `||d||`", "[spoiler]")).toBe(
      "a [spoiler] c and `||d||`",
    );
    // Markers nothing pairs, or written with backslashes, hide nothing.
    expect(replyPreview("ls | wc || true and \\|\\|c\\|\\|", "[spoiler]")).toBe(
      "ls | wc || true and \\|\\|c\\|\\|",
    );
    expect(replyPreview("||||secret|| and || || ||real||", "[spoiler]")).toBe(
      "||[spoiler] and || || [spoiler]",
    );
    expect(replyPreview("> !quoted\n> across!< lines", "[spoiler]")).toBe("[spoiler] lines");
  });

  it("writes names after spoilers are found, and none inside one", () => {
    const named = (text: string) => text.replaceAll("<@1>", "@a||b");
    expect(preview("<@1> said ||<@1> did it|| to <@1>", "[spoiler]", named)).toBe(
      "@a||b said [spoiler] to @a||b",
    );
  });

  it("cuts after spoilers are hidden, so a cut shows no part of one", () => {
    const long = `${"a".repeat(REPLY_PREVIEW_MAX_CHARS - 4)} ||secret that the cut falls inside||`;
    expect(replyPreview(long, "[spoiler]")).toBe(`${"a".repeat(REPLY_PREVIEW_MAX_CHARS - 4)} [sp`);
    expect(replyPreview(`||${"s".repeat(REPLY_PREVIEW_MAX_CHARS * 2)}|| after`, "▮")).toBe(
      "▮ after",
    );
  });

  it("leaves out code fences but keeps the code", () => {
    expect(replyPreview("look:\n```rust\nfn main() {}\n```", "[spoiler]")).toBe(
      "look: fn main() {}",
    );
  });

  it("cuts long text by characters, never inside one", () => {
    const long = "🌲".repeat(REPLY_PREVIEW_MAX_CHARS + 10);
    expect(Array.from(replyPreview(long, "")).length).toBe(REPLY_PREVIEW_MAX_CHARS);
    expect(replyPreview(long, "").endsWith("🌲")).toBe(true);
  });
});
