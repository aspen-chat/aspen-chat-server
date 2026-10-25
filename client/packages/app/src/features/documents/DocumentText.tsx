import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ErrorBoundary } from "@/features/layout/ErrorBoundary";
import { ExternalLink } from "@/features/messages/Markdown";
import { opensTooDeeply, remarkLimits, tableRow } from "@/features/messages/markdownLimits";

const REMARK_PLUGINS = [remarkGfm, remarkLimits];
const REMARK_REHYPE_OPTIONS = { handlers: { tableRow } };

/**
 * A document's headings sit one level below the page's own heading, its title, so a document
 * that opens with `# Privacy Policy` does not make a second top heading.
 */
const components: Components = {
  h1: ({ children }) => <h2>{children}</h2>,
  h2: ({ children }) => <h3>{children}</h3>,
  h3: ({ children }) => <h4>{children}</h4>,
  h4: ({ children }) => <h5>{children}</h5>,
  h5: ({ children }) => <h6>{children}</h6>,
  a: ({ href, children }) => <ExternalLink href={href}>{children}</ExternalLink>,
  // A picture is never loaded: reading the deployment's terms tells no other host the reader
  // was there. Its description stands in its place.
  img: ({ alt }) => <>{alt}</>,
};

/**
 * A deployment's document (its Privacy Policy or Terms of Use) as its administrators wrote it in
 * GitHub-flavoured Markdown. Unlike a message, a link may be named with words of its own: the
 * deployment's administrators wrote it, for their own readers. Raw HTML is ignored, only web and
 * mail addresses become links, and pictures are not loaded. A body nesting too deeply to render
 * safely, or one that fails to render, shows as its plain text.
 */
export function DocumentText({ body }: { body: string }) {
  const plain = <p>{body}</p>;
  return (
    <div className="message-body document-body">
      {opensTooDeeply(body) ? (
        plain
      ) : (
        <ErrorBoundary fallback={plain} resetKey={body}>
          <ReactMarkdown
            remarkPlugins={REMARK_PLUGINS}
            remarkRehypeOptions={REMARK_REHYPE_OPTIONS}
            components={components}
            skipHtml
          >
            {body}
          </ReactMarkdown>
        </ErrorBoundary>
      )}
    </div>
  );
}
