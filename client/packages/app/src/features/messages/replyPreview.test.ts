import { describe, expect, it } from "vitest";
import { REPLY_PREVIEW_MAX_CHARS, replyPreview } from "./replyPreview";

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
