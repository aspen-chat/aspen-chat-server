import { describe, expect, it } from "vitest";
import {
  imageUrls,
  isImageUrl,
  onlyImageLinks,
  pictureSrc,
  servedInline,
  shownSrc,
  splitInline,
} from "./images";

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

describe("pictures whose originals are served to be saved", () => {
  const original = "https://media.example.org/attachments/a";
  const copy = "https://media.example.org/attachment-previews/a";
  const preview = { url: copy, mimeType: "image/webp", width: 40, height: 20 };
  const png = { mimeType: "image/png", downloadUrl: original };
  const svg = { mimeType: "image/svg+xml", downloadUrl: original, inline: false };

  it("takes a deployment that does not say as serving every original to be shown", () => {
    expect(servedInline({})).toBe(true);
    expect(servedInline({ inline: true })).toBe(true);
    expect(servedInline({ inline: false })).toBe(false);
  });

  it("shows an original served inline, or its copy where that is preferred", () => {
    expect(pictureSrc(png, "original")).toBe(original);
    expect(pictureSrc({ ...png, preview }, "original")).toBe(original);
    expect(pictureSrc({ ...png, preview }, "preview")).toBe(copy);
    expect(pictureSrc(png, "preview")).toBe(original);
  });

  it("shows only the copy of an original served to be saved, and nothing until it has one", () => {
    expect(pictureSrc(svg, "original")).toBeUndefined();
    expect(pictureSrc(svg, "preview")).toBeUndefined();
    expect(pictureSrc({ ...svg, preview }, "original")).toBe(copy);
    expect(pictureSrc({ ...svg, preview }, "preview")).toBe(copy);
  });

  it("shows nothing of what is not a picture, or is at an address no deployment serves", () => {
    expect(pictureSrc({ ...png, mimeType: "application/pdf" }, "original")).toBeUndefined();
    expect(pictureSrc({ ...png, downloadUrl: "javascript:alert(1)" }, "original")).toBeUndefined();
    expect(
      pictureSrc({ ...svg, preview: { ...preview, url: "data:image/webp;base64,AA" } }, "preview"),
    ).toBeUndefined();
  });

  it("shows at full size what it says it shows, and the original otherwise", () => {
    expect(shownSrc({ src: original, name: "a" })).toBe(original);
    expect(shownSrc({ src: original, name: "a.svg", shown: copy })).toBe(copy);
  });
});
