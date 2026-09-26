import { describe, expect, it } from "vitest";
import { imageUrls, isImageUrl, onlyImageLinks, splitInline } from "./images";

describe("inline image cap", () => {
  it("shows up to three pictures and counts the rest", () => {
    const pictures = Array.from({ length: 5 }, (_, i) => ({ src: `s${String(i)}`, name: "" }));
    expect(splitInline(pictures.slice(0, 3))).toEqual({ shown: pictures.slice(0, 3), hidden: 0 });
    expect(splitInline(pictures)).toEqual({ shown: pictures.slice(0, 3), hidden: 2 });
    expect(splitInline([])).toEqual({ shown: [], hidden: 0 });
  });
});

describe("image links", () => {
  it("recognises image URLs by path extension only", () => {
    expect(isImageUrl("https://example.org/a/b.PNG")).toBe(true);
    expect(isImageUrl("https://example.org/a/b.jpg?w=200#x")).toBe(true);
    expect(isImageUrl("https://example.org/a/b.jpg.html")).toBe(false);
    expect(isImageUrl("https://example.org/?file=b.png")).toBe(false);
    expect(isImageUrl("not a url")).toBe(false);
  });

  it("collects distinct image links from message text, bare domains included", () => {
    expect(
      imageUrls(
        "see https://example.org/x.png and example.org/x.png then cdn.example.org/y.webp, not example.org/doc.pdf",
      ),
    ).toEqual(["https://example.org/x.png", "https://cdn.example.org/y.webp"]);
  });

  it("tells a message made only of picture links apart from one with words", () => {
    // `pics.example.org/pic` has no image extension; it stands for a link the server found to
    // be an image, which is what `isPicture` reports.
    const pictures = new Set(["https://a.example.org/x.png", "https://pics.example.org/pic"]);
    const isPicture = (url: string) => pictures.has(url);
    expect(onlyImageLinks("https://a.example.org/x.png", isPicture)).toBe(true);
    expect(
      onlyImageLinks("  https://a.example.org/x.png\n\npics.example.org/pic ", isPicture),
    ).toBe(true);
    expect(onlyImageLinks("https://a.example.org/x.png look", isPicture)).toBe(false);
    expect(
      onlyImageLinks("https://a.example.org/x.png https://c.example.org/page", isPicture),
    ).toBe(false);
    expect(onlyImageLinks("just words", isPicture)).toBe(false);
    expect(onlyImageLinks("", isPicture)).toBe(false);
  });
});
