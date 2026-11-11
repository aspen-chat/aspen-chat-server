import type { LinkPreview } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { linkPictures } from "./linkPictures";

/** A preview of a link that is itself a picture, its `url` as the server writes it. */
const picture = (url: string): LinkPreview => ({
  url,
  imageUrl: `https://media.example/${encodeURIComponent(url)}`,
  imageWidth: 640,
  imageHeight: 480,
});
/** A preview of a page with a picture of its own. */
const page = (url: string): LinkPreview => ({ ...picture(url), title: "A page" });

describe("linkPictures", () => {
  it("draws a picture link as its preview's picture and leaves its text out", () => {
    const url = "https://example.org/cat.png";
    const { pictures, cards, pictureOnly } = linkPictures(url, [picture(url)]);
    expect(pictures.map((p) => p.name)).toEqual([url]);
    expect(cards).toEqual([]);
    expect(pictureOnly).toBe(true);
  });

  it("matches a link to its preview however the link was typed", () => {
    for (const [typed, served] of [
      ["https://Example.org/cat.png", "https://example.org/cat.png"],
      ["https://example.org/猫.png", "https://example.org/%E7%8C%AB.png"],
      ["Example.org/cat.png", "https://example.org/cat.png"],
    ] as const) {
      const direct = linkPictures(typed, [picture(served)]);
      expect(direct.pictures).toHaveLength(1);
      expect(direct.cards).toEqual([]);
      expect(direct.pictureOnly).toBe(true);
      // A page named like a picture is drawn as its picture, not as a card.
      const shared = linkPictures(`look ${typed}`, [page(served)]);
      expect(shared.pictures.map((p) => p.src)).toEqual([page(served).imageUrl]);
      expect(shared.cards).toEqual([]);
      expect(shared.pictureOnly).toBe(false);
    }
  });

  it("leaves the text out for a picture behind a link with no extension", () => {
    const { pictures, pictureOnly } = linkPictures("https://Example.org", [
      picture("https://example.org/"),
    ]);
    expect(pictures).toHaveLength(1);
    expect(pictureOnly).toBe(true);
  });

  it("draws one picture for one address written two ways", () => {
    const served = "https://example.org/cat.png";
    const { pictures } = linkPictures(`https://Example.org/cat.png ${served}`, [picture(served)]);
    expect(pictures).toHaveLength(1);
  });

  it("keeps a page without a picture link as a card", () => {
    const preview = page("https://example.org/article");
    const { pictures, cards, pictureOnly } = linkPictures("https://example.org/article", [preview]);
    expect(pictures).toEqual([]);
    expect(cards).toEqual([preview]);
    expect(pictureOnly).toBe(false);
  });

  it("keeps a spoilered picture link a covered card", () => {
    const preview = picture("https://example.org/cat.png");
    const { pictures, cards, isSpoilered, pictureOnly } = linkPictures(
      "||https://Example.org/cat.png||",
      [preview],
    );
    expect(pictures).toEqual([]);
    expect(cards).toEqual([preview]);
    expect(isSpoilered(preview)).toBe(true);
    expect(pictureOnly).toBe(false);
  });
});
