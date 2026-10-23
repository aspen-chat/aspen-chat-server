import { describe, expect, it } from "vitest";
import { headLines, textLanguage } from "./textPreview";

describe("which attachments show their text", () => {
  it("highlights source files by extension, whatever type their system gave", () => {
    expect(textLanguage("main.rs", "application/octet-stream")).toBe("rs");
    expect(textLanguage("App.TSX", "")).toBe("tsx");
    expect(textLanguage("build.py", "text/x-python")).toBe("py");
  });

  it("shows prose and data as written", () => {
    expect(textLanguage("notes.txt", "text/plain;charset=utf-8")).toBeNull();
    expect(textLanguage("server.log", "application/octet-stream")).toBeNull();
    expect(textLanguage("pasted text", "text/plain")).toBeNull();
  });

  it("knows files named for what they are written in", () => {
    expect(textLanguage("Dockerfile", "")).toBe("dockerfile");
    expect(textLanguage("Makefile", "application/octet-stream")).toBe("makefile");
  });

  it("reads text types whatever their name", () => {
    expect(textLanguage("data", "application/json")).toBeNull();
    expect(textLanguage("feed.atom", "application/atom+xml")).toBe("atom");
  });

  it("leaves other files as files", () => {
    expect(textLanguage("archive.zip", "application/zip")).toBeUndefined();
    expect(textLanguage("report.pdf", "application/pdf")).toBeUndefined();
    expect(textLanguage("README", "application/octet-stream")).toBeUndefined();
  });
});

describe("a text file's first lines", () => {
  it("shows a short file whole, its final line break ending its last line", () => {
    expect(headLines("a\nb\n", true, 20)).toEqual({ text: "a\nb", more: false });
    expect(headLines("a\r\nb", true, 20)).toEqual({ text: "a\nb", more: false });
    expect(headLines("", true, 20)).toEqual({ text: "", more: false });
  });

  it("stops at the line limit and says there is more", () => {
    const lines = Array.from({ length: 21 }, (_, i) => String(i));
    expect(headLines(lines.join("\n"), true, 20)).toEqual({
      text: lines.slice(0, 20).join("\n"),
      more: true,
    });
    expect(headLines(lines.slice(0, 20).join("\n") + "\n", true, 20).more).toBe(false);
  });

  it("says there is more of a file read only in part", () => {
    expect(headLines("a\nb", false, 20)).toEqual({ text: "a\nb", more: true });
  });
});
