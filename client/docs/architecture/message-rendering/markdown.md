# Markdown

Message bodies are GitHub-flavoured Markdown, rendered by `src/features/messages/Markdown.tsx` with
`react-markdown`. No raw HTML is allowed, and unsafe schemes are dropped. Element styles are the
`message-body` rules in `styles.css`. Only messages are rendered by `Markdown.tsx`; a deployment's
documents have `DocumentText` (`documents.md`), which shares its link handling and limits.

## Limits

Parsing and rendering recurse once per level of nesting. The limits are in
`src/features/messages/markdownLimits.ts`.

| Limit               | Value     | What happens past it                           |
| ------------------- | --------- | ---------------------------------------------- |
| `MAX_NESTING`       | 32 levels | The body shows as its plain text.              |
| `MAX_TABLE_COLUMNS` | 64        | The table shows as its source in a code block. |
| `MAX_TABLE_CELLS`   | 5000      | The table shows as its source in a code block. |

- A body whose lines open `MAX_NESTING` quotes or lists at once is never parsed (`opensTooDeeply`).
- Any other too-deep body is caught once parsed, with a walk of its own (`remarkLimits`).
- A table's rows keep the cells written (`tableRow`, in place of the default handler). **Why:** the
  default pads every row to the header's width, so a wide header over many short rows would make
  millions of cells.

## Error fallbacks

- Whatever still fails to render falls back to plain text in an error boundary
  (`src/features/layout/ErrorBoundary.tsx`). There is one around the Markdown and one around each
  `MessageBody`.
- A route that fails to draw shows `RouteError` in its place (the router's `defaultErrorComponent`).

So one message cannot blank the app.

## Which links become links

- Only an absolute `http:`, `https:`, or `mailto:` address becomes a link (`messageLinkUrl`).
- A relative or protocol-relative one (`//host/share/file`) stays plain text. On a page loaded from
  a file (the desktop app's) it would be a `file:` link.
- A link is always its own address (`remarkLiteralLinks.ts`). These show as the characters they were
  written as:
  - a link its author named with words of their own (`[text](url)`);
  - a reference `[text][ref]` and its `[ref]: url` line;
  - a picture written into the text (`![alt](url)`), which is never loaded.

  The address inside is then linked as any bare address is. So `[my bank](https://evil.example)`
  cannot pass for a link to the bank.

- Autolinks (`<https://…>`) and email addresses stay links, since their text is their address.
- GFM's literal `https://…` and `www.…` links become text, linked again by the tokenizer below with
  the server's rules. GFM ends one only at whitespace or `<`, so in `||https://example.org||` it
  would take the spoiler's closing marker into the address, and the spoiler would never close.
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
(`server/app/src/link_preview/urls.rs`, list in `tlds.txt` beside it) applies the same rules, so
what renders as a link is what gets a preview. Change both together. `linkify.test.ts` reads
`tlds.txt` and fails when the `tlds` package's list differs from it, as it may after the lockfile
moves.

## Code highlighting

- Fenced code is highlighted by highlight.js (`src/features/messages/highlighter.ts`).
- It loads as its own chunk on the first code block. Its "common" grammars come with that chunk.
  Every other grammar it ships is fetched on first use.
- Token colours are the `code-*` palette tokens, mapped from `hljs-*` classes at the end of
  `styles.css`.
- Nothing is auto-detected. An unlabelled fence is plain.
- A block longer than `MAX_HIGHLIGHT_LENGTH` (4096 code units, `CodeBlock.tsx`) is plain. **Why:**
  some grammars take most of a second over a few kilobytes written to slow them.

## Spoilers

- Spoilers are `||text||` (Discord) or `>!text!<` (Reddit).
- `remarkSpoilers.ts` wraps them, and `Spoiler.tsx` renders them as a block the reader activates to
  reveal.
- Markup inside them is kept.
- While hidden, the content is `inert` and `visibility: hidden` under the block. A click on a link,
  tag, or chip in it lands on the spoiler and only reveals it, and nothing in it takes focus.
  **Why:** a hidden link that took the click would be opened unseen, and inline code, tags, and
  emoji draw boxes and pictures of their own that would show through a merely transparent text
  colour.
- A card for a link inside one is covered too (see
  [link and video cards](link-cards.md#links-inside-spoilers)).

### What makes a spoiler

- **Where markers are looked for.** In the text of one parent of the syntax tree (a paragraph, a
  heading, a table cell, emphasis). Code is not text, so a marker in code is none. A link named by
  its author's words, a picture, and a reference's definition are the text they were written as
  (`remarkLiteralLinks.ts`), backslashes and all, and the markers that text holds count.
- **A marker is two characters written as themselves.** One written with a backslash (`\|`, `\>`,
  `\!`, `\<`) or as a character reference (`&#124;`, `&vert;`, `&gt;`) is text and opens and closes
  nothing, in either spelling: `\|\|a\|\|` shows `||a||`. The parser alone knows how a character was
  written, so `remarkMarkerText` (`markerText.ts`, the first plugin) keeps that from its tokens, and
  `markersOf` is a text node's value as markers are looked for in it.
- **A table cell is the one exception, for the pipe.** GFM ends a cell at a plain `|`, so `\|` is
  the only way a cell writes one, and `\|\|text\|\|` is how a cell writes a spoiler. A pipe written
  as a character reference is text there as anywhere, and so are Reddit's markers written with
  backslashes.
- **How markers pair** (`opening`). Each spelling's openers are tried from left to right, each
  closing at the nearest closer after it:
  - something between the two that is not blank (by JavaScript's `trim`) makes a spoiler, so
    `|||a|||` hides `|a` and leaves a `|`;
  - only blank text between them (`|| ||`) makes none, both markers are text, and the search goes on
    from the end of the closer: `|| || then ||real||` hides `real`;
  - nothing between them (`||||`) makes none, the first marker is text, and the search goes on from
    the second, which may open: `||||secret||` hides `secret`;
  - no closer in the same run of text leaves the opener, and every later one of that run, open: it
    closes at the first closer in a later run of text of the same parent, whatever stands between,
    code and markup included (`||**hidden**||`), or else is text.

  Of the two spellings the one that opens first wins, and what follows a spoiler is read on the same
  way. Every step ends at least a marker further on, so a long run of pipes is read once.

- **A quote that is a spoiler.** A line starting `>!` parses as a quote, so a quote whose first
  paragraph starts with `!` and holds `!<`, each written as itself, is unwrapped and given its `>`
  back before markers are looked for.
- **A body shown as plain text** (past `MAX_NESTING`) is read as text with no Markdown, and one
  never parsed (`opensTooDeeply`) hides nothing.
- `mayHoldSpoiler` tells from a body's text alone whether it can hold one: none does without `||`,
  `!<`, or a cell's `\|\|`. `maskSpoilers` and `spoileredLinks` parse nothing else.

### One reading everywhere

Nothing else in the client decides what a spoiler is.

- `maskSpoilers` (`maskSpoilers.ts`) writes a body out with each spoiler as a word, for a line of it
  where nothing can be revealed. It reads the body with the plugins that decide where spoilers are
  (`SPOILER_REMARK_PLUGINS`, the first of what `Markdown` renders with), so it hides exactly what
  `Markdown` would.
  - What is not in a spoiler stays as written, Markdown included: each node holding no spoiler is
    its source, and one holding some is its children with the source between them.
  - Nodes the plugins make record where they were written (`extentOf`, `sourceExtent.ts`): a spoiler
    and the text left on either side share the ends of the text they were cut from.
  - The text a spoiler was cut from reads as it is shown, not as written (an escaped `\*` as `*`),
    since the tree does not say where in the source a character of text was. Markup around text
    whose place the tree does not record (a bare address GFM linked after parsing) is left out.
  - A body that can hold no spoiler is returned without being parsed, and the answers for the last
    256 bodies that were parsed are kept, so a line in a row of a list costs a parse once.

| Where a line of a message is shown      | How its spoilers are hidden                                    |
| --------------------------------------- | -------------------------------------------------------------- |
| The app's own wording of a notification | `describe`, through `maskSpoilers`                             |
| A message read out as it arrives        | `announceArrivals`, through `describe`                         |
| A reply's line                          | `replyPreview`, through `maskSpoilers`, before it is cut       |
| A search result                         | `SearchResult`, through `maskSpoilers`                         |
| Pins, saved messages, the activity feed | `ListedMessage` draws the body with `Markdown`                 |
| A linked message, an echoed reply       | `EmbeddedMessage` and `EchoedReply` draw it with `Markdown`    |
| The author's own scheduled or held text | `ScheduledMessages` and `HeldMessages`, through `maskSpoilers` |

- Names (tags, custom emoji) are written after spoilers are found, since a name may hold what would
  read as a marker.
- The server's notification text and email digest hide the same spoilers
  (`server/app/src/message/notification/spoilers.rs`). `spec/spoiler_vectors.json` holds texts both
  read alike: `maskSpoilers.test.ts` and the server's tests each run every one. A change to what
  makes a spoiler changes all three.
