import { describe, expect, it } from "vitest";
import { MAX_LINK_LENGTH, linkify } from "./linkify";

const text = (t: string) => ({ kind: "text" as const, text: t });
const link = (url: string, t = url) => ({ kind: "link" as const, url, text: t });

describe("linkify", () => {
  it("leaves plain text alone", () => {
    expect(linkify("hello there")).toEqual([text("hello there")]);
    expect(linkify("")).toEqual([]);
  });

  it("splits out http and https links, keeping the surrounding text", () => {
    expect(linkify("see https://example.org/x?y=1 and http://a.b now")).toEqual([
      text("see "),
      link("https://example.org/x?y=1"),
      text(" and "),
      link("http://a.b"),
      text(" now"),
    ]);
  });

  it("does not swallow sentence punctuation or an unmatched closing bracket", () => {
    expect(linkify("Look: https://example.org/path.")).toEqual([
      text("Look: "),
      link("https://example.org/path"),
      text("."),
    ]);
    expect(linkify("(see https://example.org/a_(b)).")).toEqual([
      text("(see "),
      link("https://example.org/a_(b)"),
      text(")."),
    ]);
  });

  it("links bare domains with a known TLD over https, keeping the text as typed", () => {
    expect(linkify("try facebook.com or GitHub.io/pages, then docs.rs/serde.")).toEqual([
      text("try "),
      link("https://facebook.com", "facebook.com"),
      text(" or "),
      link("https://GitHub.io/pages", "GitHub.io/pages"),
      text(", then "),
      link("https://docs.rs/serde", "docs.rs/serde"),
      text("."),
    ]);
    expect(linkify("localhost:5173/x and www.onet.pl")).toEqual([
      text("localhost:5173/x and "),
      link("https://www.onet.pl", "www.onet.pl"),
    ]);
  });

  it("leaves file names, emails, versions, and unknown TLDs as text", () => {
    const cases = [
      "edit main.rs and setup.py",
      "mail me at kate@example.com",
      "bump to 1.2.3 or v2.0",
      "not a site: thing.notatld",
      "ftp://x.y",
      "e.g. this",
    ];
    for (const c of cases) {
      expect(linkify(c)).toEqual([text(c)]);
    }
  });

  it("does not link inside an explicit URL twice", () => {
    expect(linkify("https://example.com/see/github.io")).toEqual([
      link("https://example.com/see/github.io"),
    ]);
  });

  it("trims long runs of brackets and punctuation", () => {
    const parens = "https://a" + ")".repeat(1990);
    expect(linkify(parens)).toEqual([link("https://a"), text(")".repeat(1990))]);
    expect(linkify("https://a" + ".)".repeat(995))).toEqual([
      link("https://a"),
      text(".)".repeat(995)),
    ]);
    expect(linkify("https://a/(" + "x".repeat(10) + "),.")).toEqual([
      link("https://a/(xxxxxxxxxx)"),
      text(",."),
    ]);
  });

  it("leaves a candidate longer than a link may be as text", () => {
    const long = "https://example.org/" + "a".repeat(MAX_LINK_LENGTH);
    expect(linkify(`${long} https://example.org/b`)).toEqual([
      text(`${long} `),
      link("https://example.org/b"),
    ]);
    const longest = "https://example.org/" + "a".repeat(MAX_LINK_LENGTH - 20);
    expect(linkify(longest)).toEqual([link(longest)]);
  });

  it("returns the same runs for the same text", () => {
    expect(linkify("see example.org")).toBe(linkify("see example.org"));
  });
});
