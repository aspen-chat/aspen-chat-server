/** Where a rail entry's community is: its deployment's domain, `null` for the user's home. */
export interface RailPlace {
  readonly domain: string | null;
  readonly communityId: string;
}

/** A community's key in the user's rail order: its id at home, `domain/id` elsewhere. */
export function railKey({ domain, communityId }: RailPlace): string {
  return domain === null ? communityId : `${domain}/${communityId}`;
}

/**
 * The rail's entries in the user's order: those `order` names as it names them, then the rest
 * as they come, which is the home's communities in the home's order and then each other
 * deployment's in its own, so a community joined anywhere shows at the end.
 */
export function arrangeRail<T extends RailPlace>(
  entries: readonly T[],
  order: readonly string[],
): T[] {
  const byKey = new Map(entries.map((entry) => [railKey(entry), entry]));
  const placed = new Set<string>();
  const arranged: T[] = [];
  for (const key of order) {
    const entry = byKey.get(key);
    if (entry !== undefined && !placed.has(key)) {
      placed.add(key);
      arranged.push(entry);
    }
  }
  for (const entry of entries) {
    if (!placed.has(railKey(entry))) {
      arranged.push(entry);
    }
  }
  return arranged;
}
