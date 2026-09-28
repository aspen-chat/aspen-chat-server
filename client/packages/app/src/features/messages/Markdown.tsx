import type { Mentions } from "@aspen/protocol";
import { isValidElement, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "@/features/messages/CodeBlock";
import { remarkBareLinks } from "@/features/messages/remarkBareLinks";
import { Mention } from "@/features/messages/Mention";
import { MentionContext } from "@/features/messages/mentionContext";
import { remarkMentions } from "@/features/messages/remarkMentions";
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
  // `remarkSpoilers` marks its spans with `data-spoiler` and `remarkMentions` with
  // `data-mention`; every other span is left as it is.
  span: ({ children, ...props }) => {
    if ("data-spoiler" in props) {
      return <Spoiler>{children}</Spoiler>;
    }
    const attributes = props as Record<string, unknown>;
    const kind = attributes["data-mention"];
    const id = attributes["data-id"];
    if (kind === "user" || kind === "role" || kind === "everyone") {
      return (
        <Mention
          kind={kind}
          id={typeof id === "string" ? id : ""}
          text={typeof children === "string" ? children : ""}
        />
      );
    }
    return <span {...props}>{children}</span>;
  },
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
 * until clicked, and tags (`<@user>`, `<@&role>`, `@everyone`) shown by name, as chips where
 * `mentions`, the message's tags as the server decided them, says they count. Raw HTML in the
 * source is ignored rather than rendered. Element styling comes from the `message-body` rules
 * in `styles.css`.
 */
export function Markdown({
  content,
  mentions = NO_MENTIONS,
  communityId = null,
}: {
  content: string;
  mentions?: Mentions;
  /** The community the message is in, where its tagged roles are found; `null` in a DM. */
  communityId?: string | null;
}) {
  return (
    <div className="message-body">
      <MentionContext.Provider value={{ mentions, communityId }}>
        <ReactMarkdown
          remarkPlugins={[remarkGfm, remarkSpoilers, remarkMentions, remarkBareLinks]}
          components={components}
          skipHtml
        >
          {content}
        </ReactMarkdown>
      </MentionContext.Provider>
    </div>
  );
}

const NO_MENTIONS: Mentions = { users: [], roles: [], everyone: false };
