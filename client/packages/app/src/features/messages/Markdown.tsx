import type { Mentions } from "@aspen/protocol";
import { type ReactNode, isValidElement, useContext } from "react";
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
import { MentionContext } from "@/features/messages/mentionContext";
import { remarkCustomEmoji } from "@/features/messages/remarkCustomEmoji";
import { remarkMentions } from "@/features/messages/remarkMentions";
import { remarkSpoilers } from "@/features/messages/remarkSpoilers";
import { Spoiler } from "@/features/messages/Spoiler";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const linkClass =
  "text-accent underline-offset-2 hover:underline focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:outline-none";

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

function ExternalLink({ href, children }: { href: string | undefined; children: ReactNode }) {
  return (
    <a href={href} target="_blank" rel="noreferrer noopener" className={linkClass}>
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
      {...(bare ? { "aria-label": format(m.reports.messageLinkFull, { url: href }), title: href } : {})}
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
 * Links take the palette accent (see `MessageLink`). `react-markdown` has already dropped any
 * URL whose scheme is not http, https, mailto, or a relative path, so `href` is safe to use.
 * Fenced blocks go through `CodeBlock` for highlighting; inline code is left to the stylesheet.
 */
const components: Components = {
  a: ({ href, children }) => <MessageLink href={href}>{children}</MessageLink>,
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
          remarkPlugins={[
            remarkGfm,
            remarkSpoilers,
            remarkMentions,
            remarkCustomEmoji,
            remarkBareLinks,
          ]}
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

/** A custom emoji in a body, resolved in the message's community. */
function CustomEmojiInMessage({ id }: { id: string }) {
  const context = useContext(MentionContext);
  return <CustomEmojiGlyph id={id} communityId={context?.communityId ?? null} />;
}
