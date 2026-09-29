/** One deployment's search results so far, newest first. */
export interface SourceResults<T extends { id: string; timestamp: string }> {
  readonly key: string;
  readonly messages: readonly T[];
  /** Whether its last page was short, so it has nothing older. */
  readonly exhausted: boolean;
}

/**
 * The results of searching several deployments at once, newest first, as far as they can be
 * shown in order: a deployment with more to give may still have messages newer than another's
 * oldest, so nothing older than the oldest message of any such deployment is shown until it has
 * given its next page.
 */
export function mergeResults<T extends { id: string; timestamp: string }>(
  sources: readonly SourceResults<T>[],
): { key: string; message: T }[] {
  const cutoff = Math.max(
    Number.NEGATIVE_INFINITY,
    ...sources
      .filter((source) => !source.exhausted)
      .map((source) => Date.parse(source.messages.at(-1)?.timestamp ?? "")),
  );
  return sources
    .flatMap((source) => source.messages.map((message) => ({ key: source.key, message })))
    .filter(({ message }) => Date.parse(message.timestamp) >= cutoff)
    .sort(
      (a, b) =>
        Date.parse(b.message.timestamp) - Date.parse(a.message.timestamp) ||
        b.message.id.localeCompare(a.message.id),
    );
}
