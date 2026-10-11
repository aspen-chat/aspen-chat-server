import type { SearchPage } from "@aspen/protocol";

/**
 * Asks a deployment for a page of a search until one holds a message or nothing is left to
 * read. A server reads only so far in one request: a search that has found nothing answers
 * with an empty page and how far back it read, and is asked again from there (`onRound` is
 * told the position each time, so the wait can say how far it has got). Stops asking once
 * `signal` is aborted, giving the page it has.
 */
export async function searchUntilFound(
  ask: (before: string | undefined) => Promise<SearchPage>,
  before: string | undefined,
  onRound: (searchedTo: string) => void,
  signal: AbortSignal,
): Promise<SearchPage> {
  let from = before;
  for (;;) {
    const page = await ask(from);
    if (page.messages.length > 0 || !page.more || page.searchedTo === null || signal.aborted) {
      return page;
    }
    from = page.searchedTo;
    onRound(page.searchedTo);
  }
}

/** The moment a position in history names: the time a UUIDv7 holds, in milliseconds. */
export function positionTime(position: string): number {
  return Number.parseInt(position.replaceAll("-", "").slice(0, 12), 16);
}
