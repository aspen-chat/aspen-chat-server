import type { Mentions } from "@aspen/protocol";
import { type ReactNode, createContext, isValidElement, memo, useContext } from "react";
import { Link } from "@tanstack/react-router";
import ReactMarkdown, { type Components } from "react-markdown";
import { parseInvite, type InviteRef } from "@/features/invites/inviteCode";
import { CodeBlock } from "@/features/messages/CodeBlock";
import { openInviteLink, useDomain } from "@/features/messages/links";
import { parseSelfLink, type KnownHosts } from "@/features/messages/selfLinks";
import { SelfLink } from "@/features/messages/SelfLink";
import { HomeClientContext } from "@/api/context";
import { useForeignDeployments } from "@/api/deploymentsContext";
import { Mention } from "@/features/messages/Mention";
import { CustomEmojiGlyph } from "@/features/emoji/CustomEmojiGlyph";
import { ErrorBoundary } from "@/features/layout/ErrorBoundary";
import { opensTooDeeply, tableRow } from "@/features/messages/markdownLimits";
import { MESSAGE_REMARK_PLUGINS } from "@/features/messages/remarkPipeline";
import { MentionContext } from "@/features/messages/mentionContext";
import { remarkAfterText } from "@/features/messages/remarkAfterText";
import { Spoiler } from "@/features/messages/Spoiler";
import { messageLinkUrl } from "@/features/layout/safeUrl";

/**
 * A link in text is underlined as well as coloured: in most palettes the accent is too near the
 * ink around it to be told apart by colour alone.
 */
const linkClass =
  "text-accent underline underline-offset-2 hover:decoration-2 focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:outline-none";

/**
 * A link in a message. A link to a deployment the user uses opens what it names here, shown by
 * those names (`SelfLink`); an Aspen invite link that names another deployment opens its
 * invite screen here (`InviteMessageLink`), whichever client made it; every other link opens
 * elsewhere.
 */
function MessageLink({ href, children }: { href: string | undefined; children: ReactNode }) {
  const hosts = useKnownHosts();
  const self = href === undefined ? null : parseSelfLink(href, hosts);
  if (self !== null && href !== undefined) {
    return <SelfLink target={self} href={href} />;
  }
  const invite = href === undefined ? null : parseInvite(href);
  if (invite?.domain != null) {
    return <InviteMessageLink invite={invite}>{children}</InviteMessageLink>;
  }
  return <ExternalLink href={href}>{children}</ExternalLink>;
}

/**
 * The addresses of the deployments the user uses: the home's (the address the app reaches it
 * at, and the page's own) and every other one they sign in to from there.
 */
function useKnownHosts(): KnownHosts {
  const home = useContext(HomeClientContext);
  const foreign = useForeignDeployments();
  return {
    home: [window.location.host, ...(home === null ? [] : [new URL(home.baseUrl).host])],
    foreign: foreign.map((d) => d.domain),
  };
}

/**
 * A link that opens elsewhere, when it is an absolute web or mail address (`messageLinkUrl`);
 * any other target, a relative or protocol-relative one included, which on a page loaded from a
 * file would become a `file:` link, leaves its text as plain text.
 */
export function ExternalLink({
  href,
  children,
}: {
  href: string | undefined;
  children: ReactNode;
}) {
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
 * becomes one (`ExternalLink`). A link is always its own address: one named by its author's
 * words, or a picture written into the text (`![alt](url)`), shows as the text it was written
 * as (`remarkLiteralLinks`), so a picture is never loaded either, which would tell whoever
 * serves it the address of everyone who reads the message. Fenced blocks go through
 * `CodeBlock` for highlighting; inline code is left to the stylesheet.
 */
const components: Components = {
  a: ({ href, children }) => <MessageLink href={href}>{children}</MessageLink>,
  // `remarkSpoilers` marks its spans with `data-spoiler`, `remarkMentions` with
  // `data-mention`, and `remarkAfterText` with `data-after-text`; every other span is left as
  // it is.
  span: ({ children, ...props }) => {
    if ("data-spoiler" in props) {
      return <Spoiler>{children}</Spoiler>;
    }
    const attributes = props as Record<string, unknown>;
    if ("data-after-text" in attributes) {
      return <AfterText />;
    }
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
 * As `components`, with every link and picture shown as its text alone, for text whose links
 * nobody vouches for (`Markdown`'s `links`).
 */
const linklessComponents: Components = {
  ...components,
  a: ({ children }) => <span>{children}</span>,
  img: ({ src, alt }) => (
    <span>{alt != null && alt !== "" ? alt : typeof src === "string" ? src : ""}</span>
  ),
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
 * in `styles.css`. With `links` false, nothing is a link: every address, written as Markdown or
 * bare, shows as text.
 *
 * A body nesting too deeply to render safely is shown as its plain text (`opensTooDeeply`
 * before parsing, `remarkLimits` after), and one that fails to render for any other reason
 * falls back to its plain text too, rather than taking the message list down with it.
 *
 * `after` follows the text (`remarkAfterText`): at the end of its last line where that is
 * text, and beside its last block where that is code, a table, or a rule, under it only where
 * the line has no room. The text is parsed again only when it, its tags, or `links` change,
 * since parsing is most of its cost; `after` reaches its place without that.
 */
export function Markdown({ after = null, ...text }: MarkdownTextProps & { after?: ReactNode }) {
  return (
    <AfterTextContext.Provider value={after}>
      <MarkdownText {...text} />
    </AfterTextContext.Provider>
  );
}

interface MarkdownTextProps {
  content: string;
  mentions?: Mentions;
  /** The community the message is in, where its tagged roles are found; `null` in a DM. */
  communityId?: string | null;
  /** Whether addresses become links; the system account's notices quote names others chose. */
  links?: boolean;
}

const MarkdownText = memo(function MarkdownText({
  content,
  mentions = NO_MENTIONS,
  communityId = null,
  links = true,
}: MarkdownTextProps) {
  const plain = (
    <p>
      {content}
      <AfterText />
    </p>
  );
  if (opensTooDeeply(content)) {
    return <div className="message-body">{plain}</div>;
  }
  return (
    <div className="message-body">
      <ErrorBoundary fallback={plain} resetKey={content}>
        <MentionContext.Provider value={{ mentions, communityId }}>
          <ReactMarkdown
            remarkPlugins={REMARK_PLUGINS}
            remarkRehypeOptions={REMARK_REHYPE_OPTIONS}
            components={links ? components : linklessComponents}
            skipHtml
          >
            {content}
          </ReactMarkdown>
        </MentionContext.Provider>
      </ErrorBoundary>
    </div>
  );
});

/** What follows the text, for `AfterText` to place. */
const AfterTextContext = createContext<ReactNode>(null);

/**
 * What follows the text (`Markdown`'s `after`), where `remarkAfterText` placed it. Each mark
 * keeps a little room before it; a mark that is hidden (a grouped message's time while the
 * pointer is elsewhere) has no box, so neither takes any room nor makes the text a line longer.
 */
function AfterText() {
  const after = useContext(AfterTextContext);
  if (after === null) {
    return null;
  }
  return (
    <span className="message-after inline-flex flex-wrap items-baseline font-normal *:ms-1">
      {after}
    </span>
  );
}

/**
 * A message's pipeline, then the marks that follow its text (`remarkAfterText`), which only
 * rendering places.
 */
const REMARK_PLUGINS = [...MESSAGE_REMARK_PLUGINS, remarkAfterText];

/** Table rows keep the cells written (`tableRow`). */
const REMARK_REHYPE_OPTIONS = { handlers: { tableRow } };

const NO_MENTIONS: Mentions = { users: [], roles: [], everyone: false };

/** A custom emoji in a body, resolved in the message's community. */
function CustomEmojiInMessage({ id }: { id: string }) {
  const context = useContext(MentionContext);
  return <CustomEmojiGlyph id={id} communityId={context?.communityId ?? null} />;
}
