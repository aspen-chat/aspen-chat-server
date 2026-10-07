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

  it("links explicit URLs and bare domains, but not code", () => {
    const html = render("https://example.org/d or github.io/pages, not `main.rs` or `x.com`");
    expect(html).toContain('href="https://example.org/d"');
    expect(html).toContain('href="https://github.io/pages"');
    expect(html).not.toContain('href="https://x.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noreferrer noopener"');
  });

  it("shows a link named by its author's words as written, linking only its address", () => {
    const html = render("[my bank](https://evil.example/login) and <https://ok.example>");
    expect(html).toContain("[my bank](");
    expect(html).toContain('<a href="https://evil.example/login"');
    expect(html).toContain(">https://evil.example/login</a>)");
    expect(html).not.toContain(">my bank</a>");
    expect(html).toContain('href="https://ok.example/"');

    const spoofed = render("[https://good.example](https://evil.example)");
    expect(spoofed).toContain("[<span>https://good.example](https://evil.example)</span>");
    expect(spoofed).not.toContain("<a ");

    const named = render("[a good site](https://evil.example)");
    expect(named).toContain(">https://evil.example</a>)");
    expect(named).not.toContain(">a good site</a>");
  });

  it("shows references and their definitions as written", () => {
    const html = render("see [the docs][d]\n\n[d]: https://evil.example/d");
    expect(html).toContain("see [the docs][d]");
    expect(html).toContain("[d]: ");
    expect(html).toContain('href="https://evil.example/d"');
    expect(html).not.toContain(">the docs</a>");
  });

  it("keeps markup outside a literal link, and tags and spoilers inside it", () => {
    const html = render("**bold** [||secret||](https://x.example) _it_");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<em>it</em>");
    expect(html).toContain('role="button"');
    expect(html).toContain('href="https://x.example/"');
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
    expect(html).not.toContain('href="javascript:');
    expect(html).toContain("[x](javascript:alert(1))");
  });

  it("links only absolute web and mail addresses, so nothing becomes a file link", () => {
    const html = render(
      "<//attacker/share/payload.exe> [b](/etc/passwd) <file:///etc/passwd> <mailto:x@example.org>",
    );
    expect(html).not.toContain('href="//attacker');
    expect(html).not.toContain('href="file:');
    expect((html.match(/<a /g) ?? []).length).toBe(1);
    expect(html).toContain('href="mailto:x@example.org"');
  });

  it("shows pictures written into the text as written, never loading them", () => {
    const html = render("![a cat](https://tracker.example/cat.png) ![](//attacker/x.png)");
    expect(html).not.toContain("<img");
    expect(html).toContain("![a cat](");
    expect(html).toContain('href="https://tracker.example/cat.png"');
    expect(html).not.toContain('href="//attacker');
  });
});
