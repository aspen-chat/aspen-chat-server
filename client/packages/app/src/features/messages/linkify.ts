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
const TRAILING = /[.,;:!?'"]+$/;

/**
 * Splits message text into plain runs and links. Explicit URLs and bare domains with a known
 * TLD both link, the latter over `https`. Sentence punctuation after a link is left as text,
 * and a closing bracket is only kept when the link itself opened one, so a parenthesised URL
 * such as `(see https://example.org/a_(b))` ends at the right place.
 */
export function linkify(content: string): TextRun[] {
  const runs: TextRun[] = [];
  let last = 0;
  for (const match of content.matchAll(CANDIDATE)) {
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

function trimTrailing(text: string): string {
  let trimmed = text.replace(TRAILING, "");
  while (trimmed.endsWith(")") && count(trimmed, "(") < count(trimmed, ")")) {
    trimmed = trimmed.slice(0, -1).replace(TRAILING, "");
  }
  return trimmed;
}

function count(text: string, char: string): number {
  let n = 0;
  for (const c of text) {
    if (c === char) {
      n += 1;
    }
  }
  return n;
}
