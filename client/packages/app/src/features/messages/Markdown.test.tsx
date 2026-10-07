import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";

const render = (content: string) => renderToStaticMarkup(<Markdown content={content} />);

describe("Markdown", () => {
  it("renders GitHub-flavoured Markdown", () => {
    const html = render("**bold** _it_ `x < y` ~~gone~~\n\n- one\n- two");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<em>it</em>");
    expect(html).toContain("<code>x &lt; y</code>");
    expect(html).toContain("<del>gone</del>");
    expect(html).toContain("<li>one</li>");
  });

  it("links explicit URLs, Markdown links, and bare domains, but not code", () => {
    const html = render(
      "[docs](https://example.org/d) or github.io/pages, not `main.rs` or `x.com`",
    );
    expect(html).toContain('href="https://example.org/d"');
    expect(html).toContain('href="https://github.io/pages"');
    expect(html).not.toContain('href="https://x.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noreferrer noopener"');
  });

  it("hides spoilers in either spelling until revealed, keeping markup inside them", () => {
    const html = render("the killer is ||**the butler**|| obviously");
    expect(html).toContain('role="button"');
    expect(html).toContain('aria-label="Spoiler, activate to reveal"');
    expect(html).toContain('<span aria-hidden="true"><strong>the butler</strong></span>');
    expect(html).not.toContain("||");
    expect(html).toContain("the killer is ");
    expect(html).toContain(" obviously");

    const reddit = render(">!hidden!< and >!also hidden!<");
    expect(reddit).not.toContain("<blockquote");
    expect((reddit.match(/role="button"/g) ?? []).length).toBe(2);
    expect(reddit).toContain(">hidden</span>");

    expect(render("a || b")).toContain("a || b");
    expect(render("|| ||")).toContain("|| ||");
    expect(render("`||code||`")).toContain("<code>||code||</code>");
    expect(render("> a quote")).toContain("<blockquote>");
  });

  it("ignores raw HTML and unsafe schemes", () => {
    const html = render("<img src=x onerror=alert(1)> [x](javascript:alert(1))");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("javascript:");
  });

  it("links only absolute web and mail addresses, so nothing becomes a file link", () => {
    const html = render(
      "[a](//attacker/share/payload.exe) [b](/etc/passwd) [c](payload.exe) [d](file:///etc/passwd) [e](mailto:x@example.org)",
    );
    expect(html).not.toContain('attacker/share"');
    expect((html.match(/<a /g) ?? []).length).toBe(1);
    expect(html).toContain('href="mailto:x@example.org"');
    expect(html).toContain("<span>a</span>");
  });

  it("shows pictures written into the text as links, never loading them", () => {
    const html = render("![a cat](https://tracker.example/cat.png) ![](//attacker/x.png)");
    expect(html).not.toContain("<img");
    expect(html).toContain('href="https://tracker.example/cat.png"');
    expect(html).toContain(">a cat</a>");
    expect(html).not.toContain('href="//attacker');
  });

  it("shows a body nesting too deeply to render as its plain text", () => {
    for (const content of [">".repeat(10000), "1. ".repeat(1400), "- ".repeat(5000)]) {
      const html = render(content);
      expect(html).not.toContain("<blockquote");
      expect(html).not.toContain("<li");
      expect(html).toContain(content.trim().slice(0, 40).replaceAll(">", "&gt;"));
    }
    // Nesting built by indenting is caught once parsed.
    const indented = Array.from({ length: 40 }, (_, i) => `${"  ".repeat(i)}- a`).join("\n");
    expect(render(indented)).not.toContain("<li");
    // Nesting a person writes is drawn as Markdown.
    expect(render("> > > quoted\n\n- a\n  - b\n    - c")).toContain("<blockquote>");
    expect((render("- a\n  - b\n    - c").match(/<li>/g) ?? []).length).toBe(3);
  });

  it("draws a table's rows as written, and a table too large to draw as its source", () => {
    const small = render("| a | b | c |\n| - | :-: | - |\n| 1 |\n| 1 | 2 | 3 |");
    expect(small).toContain("<th>a</th>");
    expect(small).toContain('<th style="text-align:center">b</th>');
    expect(small).toContain("<tr><td>1</td></tr>");
    expect((small.match(/<td/g) ?? []).length).toBe(4);

    // A wide header over many short rows would otherwise be padded to millions of cells.
    const wide = "a|".repeat(1600) + "\n" + "-|".repeat(1600) + "\n" + "a\n".repeat(1690);
    const wideHtml = render(wide);
    expect(wideHtml).not.toContain("<table");
    expect(wideHtml).toContain("<pre><code>a|a|");

    const row = "|" + " a |".repeat(10) + "\n";
    const big = row + "|" + " - |".repeat(10) + "\n" + row.repeat(500);
    expect(render(big)).not.toContain("<table");
    expect(render(row + "|" + " - |".repeat(10) + "\n" + row.repeat(400))).toContain("<table");
  });
});
