import tlds from "tlds";

/** A run of message text: either plain, or a link to open. */
export type TextRun = { kind: "text"; text: string } | { kind: "link"; url: string; text: string };

/** Every TLD IANA delegates, lower case, for recognising bare domains such as `github.io`. */
const KNOWN_TLDS: ReadonlySet<string> = new Set(tlds);

/**
 * TLDs that are far more often file extensions in chat about software (`main.rs`, `setup.py`)
 * than domains. A bare two-label token with one of these is left as text; a port, a path, or a
 * `www.` prefix makes the intent clear and links it anyway.
 */
const EXTENSION_LIKE_TLDS: ReadonlySet<string> = new Set(["rs", "py", "sh", "md", "pl", "ps"]);

/**
 * Candidate links, leftmost first. The first branch is an explicit `http` or `https` URL. The
 * second is a bare domain: dot-separated labels ending in a TLD, then an optional port and path.
 * It must not follow a word character, `@`, `.`, `/`, or `-`, which rules out email addresses,
 * the tails of URLs already matched, and version numbers.
 */
const CANDIDATE =
  /(https?:\/\/[^\s<>]+)|(?<![\w@./-])((?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+([a-z]{2,63}))(:\d{1,5})?((?:\/[^\s<>]*)?)/gi;
/** Punctuation that usually belongs to the sentence rather than the link when it ends one. */
const TRAILING: ReadonlySet<string> = new Set([".", ",", ";", ":", "!", "?", "'", '"']);

/**
 * The longest candidate that becomes a link; a longer one is left as text. The server's preview
 * extractor (`server/app/src/link_preview/urls.rs`) skips longer ones too, so what renders as a
 * link is what gets a preview.
 */
export const MAX_LINK_LENGTH = 2048;

/** How many texts' runs `linkify` keeps, so a message drawn again is not scanned again. */
const CACHE_SIZE = 512;
const cache = new Map<string, readonly TextRun[]>();

/**
 * Splits message text into plain runs and links. Explicit URLs and bare domains with a known
 * TLD both link, the latter over `https`. Sentence punctuation after a link is left as text,
 * and a closing bracket is only kept when the link itself opened one, so a parenthesised URL
 * such as `(see https://example.org/a_(b))` ends at the right place. A candidate longer than
 * `MAX_LINK_LENGTH` stays text. The runs of the last `CACHE_SIZE` texts are kept and shared, so
 * they must not be changed.
 */
export function linkify(content: string): readonly TextRun[] {
  const cached = cache.get(content);
  if (cached !== undefined) {
    return cached;
  }
  const runs = scan(content);
  if (cache.size >= CACHE_SIZE) {
    const oldest = cache.keys().next();
    if (oldest.done !== true) {
      cache.delete(oldest.value);
    }
  }
  cache.set(content, runs);
  return runs;
}

function scan(content: string): readonly TextRun[] {
  const runs: TextRun[] = [];
  let last = 0;
  for (const match of content.matchAll(CANDIDATE)) {
    if (match[0].length > MAX_LINK_LENGTH) {
      continue;
    }
    const [, explicit, domain, tld, port, path] = match;
    let text: string;
    if (explicit !== undefined) {
      text = explicit;
    } else {
      if (domain === undefined || tld === undefined || !isLinkableDomain(domain, tld, port, path)) {
        continue;
      }
      text = domain + (port ?? "") + (path ?? "");
    }
    text = trimTrailing(text);
    if (text.length === 0) {
      continue;
    }
    const start = match.index;
    if (start > last) {
      runs.push({ kind: "text", text: content.slice(last, start) });
    }
    runs.push({
      kind: "link",
      url: explicit === undefined ? `https://${text}` : text,
      text,
    });
    last = start + text.length;
  }
  if (last < content.length) {
    runs.push({ kind: "text", text: content.slice(last) });
  }
  return runs;
}

function isLinkableDomain(
  domain: string,
  tld: string,
  port: string | undefined,
  path: string | undefined,
): boolean {
  const lower = tld.toLowerCase();
  if (!KNOWN_TLDS.has(lower)) {
    return false;
  }
  if (!EXTENSION_LIKE_TLDS.has(lower)) {
    return true;
  }
  const explicitEnough =
    port !== undefined || (path !== undefined && path.length > 0) || /^www\./i.test(domain);
  return explicitEnough;
}

/**
 * `text` without the sentence punctuation that ends it, nor a closing bracket it did not open
 * (with the punctuation before that), in one pass from each end.
 */
function trimTrailing(text: string): string {
  let unmatched = 0;
  for (const c of text) {
    if (c === ")") {
      unmatched += 1;
    } else if (c === "(") {
      unmatched -= 1;
    }
  }
  let end = text.length;
  for (;;) {
    while (end > 0 && TRAILING.has(text.charAt(end - 1))) {
      end -= 1;
    }
    if (end > 0 && text.charAt(end - 1) === ")" && unmatched > 0) {
      unmatched -= 1;
      end -= 1;
    } else {
      return text.slice(0, end);
    }
  }
}
