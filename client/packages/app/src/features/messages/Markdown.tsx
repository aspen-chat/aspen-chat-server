import { isValidElement, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "@/features/messages/CodeBlock";
import { remarkBareLinks } from "@/features/messages/remarkBareLinks";
import { remarkSpoilers } from "@/features/messages/remarkSpoilers";
import { Spoiler } from "@/features/messages/Spoiler";

/**
 * Links open elsewhere and take the palette accent. `react-markdown` has already dropped any
 * URL whose scheme is not http, https, mailto, or a relative path, so `href` is safe to use.
 * Fenced blocks go through `CodeBlock` for highlighting; inline code is left to the stylesheet.
 */
const components: Components = {
  a: ({ href, children }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      className="text-accent underline-offset-2 hover:underline focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:outline-none"
    >
      {children}
    </a>
  ),
  // `remarkSpoilers` marks its spans with `data-spoiler`; every other span is left as it is.
  span: ({ children, ...props }) =>
    "data-spoiler" in props ? <Spoiler>{children}</Spoiler> : <span {...props}>{children}</span>,
  pre: ({ children }) => {
    const code = fencedCode(children);
    if (code === null) {
      return <pre>{children}</pre>;
    }
    return <CodeBlock code={code.text} language={code.language} />;
  },
};

/**
 * The text and language of a fenced block, read from the `<code>` element react-markdown
 * places inside `<pre>`, whose class is `language-<fence name>` when the fence named one.
 */
function fencedCode(children: ReactNode): { text: string; language: string | null } | null {
  if (!isValidElement<{ className?: string; children?: ReactNode }>(children)) {
    return null;
  }
  const { className, children: inner } = children.props;
  if (typeof inner !== "string") {
    return null;
  }
  const language = /(?:^|\s)language-([^\s]+)/.exec(className ?? "")?.[1] ?? null;
  return { text: inner.replace(/\n$/, ""), language };
}

/**
 * A message body as GitHub-flavoured Markdown: emphasis, code, lists, quotes, tables, and
 * links, with bare domains linked too, and `||spoilers||` (or Reddit's `>!spoilers!<`) hidden
 * until clicked. Raw HTML in the source is ignored rather than rendered.
 * Element styling comes from the `message-body` rules in `styles.css`.
 */
export function Markdown({ content }: { content: string }) {
  return (
    <div className="message-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkSpoilers, remarkBareLinks]}
        components={components}
        skipHtml
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
