# Markdown

Message bodies are GitHub-flavoured Markdown, rendered by `src/features/messages/Markdown.tsx` with
`react-markdown`. No raw HTML is allowed, and unsafe schemes are dropped. Element styles are the
`message-body` rules in `styles.css`. Only messages are rendered by `Markdown.tsx`.

## Limits

Parsing and rendering recurse once per level of nesting. The limits are in
`src/features/messages/markdownLimits.ts`.

| Limit | Value | What happens past it |
| --- | --- | --- |
| `MAX_NESTING` | 32 levels | The body shows as its plain text. |
| `MAX_TABLE_COLUMNS` | 64 | The table shows as its source in a code block. |
| `MAX_TABLE_CELLS` | 5000 | The table shows as its source in a code block. |

- A body whose lines open `MAX_NESTING` quotes or lists at once is never parsed (`opensTooDeeply`).
- Any other too-deep body is caught once parsed, with a walk of its own (`remarkLimits`).
- A table's rows keep the cells written (`tableRow`, in place of the default handler). **Why:** the
  default pads every row to the header's width, so a wide header over many short rows would make
  millions of cells.

## Error fallbacks

- Whatever still fails to render falls back to plain text in an error boundary
  (`src/features/layout/ErrorBoundary.tsx`). There is one around the Markdown and one around each
  `MessageBody`.
- A route that fails to draw shows `RouteError` in its place (the router's
  `defaultErrorComponent`).

So one message cannot blank the app.

## Which links become links

- Only an absolute `http:`, `https:`, or `mailto:` address becomes a link (`messageLinkUrl`).
- A relative or protocol-relative one (`//host/share/file`) stays plain text. On a page loaded from a
  file (the desktop app's) it would be a `file:` link.
- A link is always its own address (`remarkLiteralLinks.ts`). These show as the characters they were
  written as:
  - a link its author named with words of their own (`[text](url)`);
  - a reference `[text][ref]` and its `[ref]: url` line;
  - a picture written into the text (`![alt](url)`), which is never loaded.

  The address inside is then linked as any bare address is. So `[my bank](https://evil.example)`
  cannot pass for a link to the bank.
- Autolinks (`<https://…>`) and GFM's literal `https://…` and `www.…` stay links, since their text is
  their address.
- The system account's notices quote names others chose (a community's, a person's). `MessageBody`
  renders its messages with `links` off: every address in them, Markdown or bare, shows as text, and
  the server fetches no previews for them.

## Linkifying bare addresses

On top of GFM's own autolinks, bare domains are linked by the tokenizer in
`src/features/messages/linkify.ts`:

- explicit `http(s)` URLs;
- bare domains whose TLD is on IANA's list (the `tlds` package);
- a handful of TLDs that double as source-file extensions link only with a port, a path, or `www.`.
  The set is a constant in that file.

Rules:

- A candidate longer than `MAX_LINK_LENGTH` (2048) stays text.
- Trailing punctuation and unmatched brackets are trimmed in one pass.
- The runs of recent texts are kept, so a message drawn again is not scanned again. `Markdown`
  itself is drawn again only when its content, tags, or community change.

**Keep in step with the server.** The server's preview extractor
(`server/app/src/link_preview/urls.rs`, list in `tlds.txt` beside it) applies the same rules, so what
renders as a link is what gets a preview. Change both together.

## Code highlighting

- Fenced code is highlighted by highlight.js (`src/features/messages/highlighter.ts`).
- It loads as its own chunk on the first code block. Its "common" grammars come with that chunk.
  Every other grammar it ships is fetched on first use.
- Token colours are the `code-*` palette tokens, mapped from `hljs-*` classes at the end of
  `styles.css`.
- Nothing is auto-detected. An unlabelled fence is plain.
- A block longer than `MAX_HIGHLIGHT_LENGTH` (4096 code units, `CodeBlock.tsx`) is plain.
  **Why:** some grammars take most of a second over a few kilobytes written to slow them.

## Spoilers

- Spoilers are `||text||` (Discord) or `>!text!<` (Reddit).
- `remarkSpoilers.ts` wraps them, and `Spoiler.tsx` renders them as a block the reader activates to
  reveal.
- Markup inside them is kept.
