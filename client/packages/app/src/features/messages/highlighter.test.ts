import { describe, expect, it } from "vitest";
import { ensureLanguage, highlight, isReady } from "./highlighter";

describe("highlighter", () => {
  it("highlights an eager language at once, resolving registered aliases", () => {
    expect(isReady("rust")).toBe(true);
    expect(isReady("js")).toBe(true);
    const html = highlight("let x = 1;", "js");
    expect(html).toContain('class="hljs-keyword">let</span>');
    expect(html).toContain('class="hljs-number">1</span>');
  });

  it("escapes the code it marks up", () => {
    expect(highlight('"<b>"', "js")).toContain("&lt;b&gt;");
  });

  it("fetches a grammar outside the eager set on demand", async () => {
    expect(isReady("elixir")).toBe(false);
    expect(await ensureLanguage("ex")).toBe(true);
    expect(isReady("elixir")).toBe(true);
    expect(highlight("defmodule A do end", "elixir")).toContain("hljs-keyword");
  });

  it("reports languages highlight.js does not have", async () => {
    expect(await ensureLanguage("not-a-language")).toBe(false);
    expect(highlight("x", "not-a-language")).toBeNull();
  });
});
