import { describe, expect, it } from "vitest";
import { normalizedUrl, spoileredLinks } from "./spoileredLinks";

const links = (content: string) => [...spoileredLinks(content)].sort();

describe("spoileredLinks", () => {
  it("finds links inside either spelling of spoiler, as the server writes preview URLs", () => {
    expect(links("see ||https://aspen.example|| and https://open.example")).toEqual([
      "https://aspen.example/",
    ]);
    expect(links(">!**twist** at github.io/pages!<")).toEqual(["https://github.io/pages"]);
    expect(links("||<https://auto.example/a?b=1>||")).toEqual(["https://auto.example/a?b=1"]);
    expect(links("||www.example.org/x||")).toEqual(["https://www.example.org/x"]);
  });

  it("finds nothing where no spoiler holds a link", () => {
    expect(links("https://aspen.example")).toEqual([]);
    expect(links("||no link here||")).toEqual([]);
    expect(links("`||https://code.example||`")).toEqual([]);
    expect(links("a || b https://aspen.example")).toEqual([]);
    expect(links("[||named||](https://named.example)")).toEqual([]);
  });

  it("normalizes as the url crate serializes", () => {
    expect(normalizedUrl("https://Aspen.Example")).toBe("https://aspen.example/");
    expect(normalizedUrl("not a url")).toBeNull();
  });
});
