import { useEffect, useState } from "react";
import type * as HighlighterModule from "@/features/messages/highlighter";

type Highlighter = typeof HighlighterModule;

let highlighterModule: Promise<Highlighter> | null = null;

/** The highlighter chunk, fetched once, the first time any code block asks for it. */
function loadHighlighter(): Promise<Highlighter> {
  highlighterModule ??= import("@/features/messages/highlighter");
  return highlighterModule;
}

/**
 * The longest block, in UTF-16 code units, that is highlighted. Some grammars take most of a
 * second over a block of a few kilobytes written to slow them, and highlighting runs on the one
 * thread the page has.
 */
export const MAX_HIGHLIGHT_LENGTH = 4096;

/**
 * A fenced code block. Renders the code as plain text at once and swaps in highlighted markup
 * once the highlighter, and the block's grammar if it is not in the eager set, have loaded.
 * A fence without a language, with one highlight.js does not know, or longer than
 * `MAX_HIGHLIGHT_LENGTH` stays plain.
 */
export function CodeBlock({ code, language }: { code: string; language: string | null }) {
  // The markup made, with the code and language it was made for, so a block whose code
  // changed shows the new code plainly until its own markup is made.
  const [made, setMade] = useState<{ code: string; language: string; html: string } | null>(null);

  useEffect(() => {
    if (language === null || code.length > MAX_HIGHLIGHT_LENGTH) {
      return;
    }
    let cancelled = false;
    void loadHighlighter()
      .then(async (highlighter) => {
        if (!(await highlighter.ensureLanguage(language))) {
          return null;
        }
        return highlighter.highlight(code, language);
      })
      .then((html) => {
        if (!cancelled && html !== null) {
          setMade({ code, language, html });
        }
      })
      .catch(() => {
        // Highlighting is decoration; the plain block is already on screen.
      });
    return () => {
      cancelled = true;
    };
  }, [code, language]);

  const html = made?.code === code && made.language === language ? made.html : null;
  return (
    <pre>
      {html === null ? (
        <code>{code}</code>
      ) : (
        // highlight.js escapes the code and emits only its own spans, so this is markup we
        // produced from text, not text from the server rendered as markup.
        <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
      )}
    </pre>
  );
}
