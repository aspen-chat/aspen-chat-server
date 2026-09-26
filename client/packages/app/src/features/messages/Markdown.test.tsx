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
});
