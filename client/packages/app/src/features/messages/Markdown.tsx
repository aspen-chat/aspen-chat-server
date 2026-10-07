import type { Mentions } from "@aspen/protocol";
import { type ReactNode, isValidElement, memo, useContext } from "react";
import { ChatTextIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { parseInvite, type InviteRef } from "@/features/invites/inviteCode";
import { CodeBlock } from "@/features/messages/CodeBlock";
import {
  messageLink,
  openInviteLink,
  parseMessageUrl,
  useDomain,
  type MessageUrl,
} from "@/features/messages/links";
import { HomeClientContext } from "@/api/context";
import { useForeignDeployments } from "@/api/deploymentsContext";
import { remarkBareLinks } from "@/features/messages/remarkBareLinks";
import { Mention } from "@/features/messages/Mention";
import { CustomEmojiGlyph } from "@/features/emoji/CustomEmojiGlyph";
import { ErrorBoundary } from "@/features/layout/ErrorBoundary";
import { opensTooDeeply, remarkLimits, tableRow } from "@/features/messages/markdownLimits";
import { MentionContext } from "@/features/messages/mentionContext";
import { remarkCustomEmoji } from "@/features/messages/remarkCustomEmoji";
import { remarkMentions } from "@/features/messages/remarkMentions";
import { remarkSpoilers } from "@/features/messages/remarkSpoilers";
import { Spoiler } from "@/features/messages/Spoiler";
import { messageLinkUrl } from "@/features/layout/safeUrl";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A link in text is underlined as well as coloured: in most palettes the accent is too near the
 * ink around it to be told apart by colour alone.
 */
const linkClass =
  "text-accent underline underline-offset-2 hover:decoration-2 focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:outline-none";

/**
 * A link in a message. An Aspen invite link that names its deployment opens its invite screen
 * here (`InviteMessageLink`), whichever client made it, and a link to a message of a deployment
 * the user uses opens it here (`LinkToMessage`); every other link opens elsewhere.
 */
function MessageLink({ href, children }: { href: string | undefined; children: ReactNode }) {
  const invite = href === undefined ? null : parseInvite(href);
  if (invite?.domain != null) {
    return <InviteMessageLink invite={invite}>{children}</InviteMessageLink>;
  }
  const message = href === undefined ? null : parseMessageUrl(href);
  if (message !== null) {
    return (
      <LinkToMessage target={message} href={href ?? ""}>
        {children}
      </LinkToMessage>
    );
  }
  return <ExternalLink href={href}>{children}</ExternalLink>;
}

/**
 * A link that opens elsewhere, when it is an absolute web or mail address (`messageLinkUrl`);
 * any other target, a relative or protocol-relative one included, which on a page loaded from a
 * file would become a `file:` link, leaves its text as plain text.
 */
function ExternalLink({ href, children }: { href: string | undefined; children: ReactNode }) {
  const target = messageLinkUrl(href);
  if (target === undefined) {
    return <span>{children}</span>;
  }
  return (
    <a href={target} target="_blank" rel="noreferrer noopener" className={linkClass}>
      {children}
    </a>
  );
}

/**
 * A link to a message, opened here when it is on the user's home deployment (the address the
 * app reaches it at, or the page's own) or another they use, and elsewhere otherwise. Written
 * as its bare address, it shows as a short label, since the message itself shows beneath.
 */
function LinkToMessage({
  target,
  href,
  children,
}: {
  target: MessageUrl;
  href: string;
  children: ReactNode;
}) {
  const m = useMessages();
  const home = useContext(HomeClientContext);
  const foreign = useForeignDeployments();
  const homeHosts = [window.location.host, ...(home === null ? [] : [new URL(home.baseUrl).host])];
  const domain = homeHosts.includes(target.host)
    ? null
    : foreign.find((d) => d.domain === target.host)?.domain;
  if (domain === undefined) {
    return <ExternalLink href={href}>{children}</ExternalLink>;
  }
  const bare = textOf(children) === href;
  return (
    <Link
      {...messageLink({ domain, community: target.community }, target.channel, target.message)}
      {...(bare
        ? { "aria-label": format(m.reports.messageLinkFull, { url: href }), title: href }
        : {})}
      className={
        bare
          ? "inline-flex items-baseline gap-0.5 rounded bg-accent-soft px-1 text-accent-strong outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
          : linkClass
      }
    >
      {bare ? (
        <>
          <ChatTextIcon size={14} aria-hidden="true" className="self-center" />
          {m.reports.messageLinkLabel}
        </>
      ) : (
        children
      )}
    </Link>
  );
}

/** The plain text of a link's children, or `null` when they hold more than text. */
function textOf(children: ReactNode): string | null {
  if (typeof children === "string") {
    return children;
  }
  if (Array.isArray(children) && children.every((child) => typeof child === "string")) {
    return children.join("");
  }
  return null;
}

function InviteMessageLink({ invite, children }: { invite: InviteRef; children: ReactNode }) {
  const current = useDomain();
  return (
    <Link {...openInviteLink(invite, current)} className={linkClass}>
      {children}
    </Link>
  );
}

/**
 * Links take the palette accent (see `MessageLink`), and only an absolute web or mail address
 * becomes one (`ExternalLink`). A picture written into the text (`![alt](url)`) is shown as a
 * link to it, named by its alt text, never loaded: loading it would tell whoever serves it the
 * address of everyone who reads the message. Fenced blocks go through `CodeBlock` for
 * highlighting; inline code is left to the stylesheet.
 */
const components: Components = {
  a: ({ href, children }) => <MessageLink href={href}>{children}</MessageLink>,
  img: ({ src, alt }) => {
    const href = typeof src === "string" ? src : undefined;
    return <MessageLink href={href}>{alt != null && alt !== "" ? alt : href}</MessageLink>;
  },
  // `remarkSpoilers` marks its spans with `data-spoiler` and `remarkMentions` with
  // `data-mention`; every other span is left as it is.
  span: ({ children, ...props }) => {
    if ("data-spoiler" in props) {
      return <Spoiler>{children}</Spoiler>;
    }
    const attributes = props as Record<string, unknown>;
    const emoji = attributes["data-emoji"];
    if (typeof emoji === "string") {
      return <CustomEmojiInMessage id={emoji} />;
    }
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
 *
 * A body nesting too deeply to render safely is shown as its plain text (`opensTooDeeply`
 * before parsing, `remarkLimits` after), and one that fails to render for any other reason
 * falls back to its plain text too, rather than taking the message list down with it. It is
 * drawn again only when what it is given changes, since parsing is most of its cost.
 */
export const Markdown = memo(function Markdown({
  content,
  mentions = NO_MENTIONS,
  communityId = null,
}: {
  content: string;
  mentions?: Mentions;
  /** The community the message is in, where its tagged roles are found; `null` in a DM. */
  communityId?: string | null;
}) {
  const plain = <p>{content}</p>;
  if (opensTooDeeply(content)) {
    return <div className="message-body">{plain}</div>;
  }
  return (
    <div className="message-body">
      <ErrorBoundary fallback={plain} resetKey={content}>
        <MentionContext.Provider value={{ mentions, communityId }}>
          <ReactMarkdown
            remarkPlugins={[
              remarkGfm,
              remarkLimits,
              remarkSpoilers,
              remarkMentions,
              remarkCustomEmoji,
              remarkBareLinks,
            ]}
            remarkRehypeOptions={REMARK_REHYPE_OPTIONS}
            components={components}
            skipHtml
          >
            {content}
          </ReactMarkdown>
        </MentionContext.Provider>
      </ErrorBoundary>
    </div>
  );
});

/** Table rows keep the cells written (`tableRow`). */
const REMARK_REHYPE_OPTIONS = { handlers: { tableRow } };

const NO_MENTIONS: Mentions = { users: [], roles: [], everyone: false };

/** A custom emoji in a body, resolved in the message's community. */
function CustomEmojiInMessage({ id }: { id: string }) {
  const context = useContext(MentionContext);
  return <CustomEmojiGlyph id={id} communityId={context?.communityId ?? null} />;
}
