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
 * A fenced code block. Renders the code as plain text at once and swaps in highlighted markup
 * once the highlighter, and the block's grammar if it is not in the eager set, have loaded.
 * A fence without a language, or with one highlight.js does not know, stays plain.
 */
export function CodeBlock({ code, language }: { code: string; language: string | null }) {
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    if (language === null) {
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
      .then((result) => {
        if (!cancelled) {
          setHtml(result);
        }
      })
      .catch(() => {
        // Highlighting is decoration; the plain block is already on screen.
      });
    return () => {
      cancelled = true;
    };
  }, [code, language]);

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
