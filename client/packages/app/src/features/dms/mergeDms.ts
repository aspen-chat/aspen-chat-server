/** One deployment's DMs, in its own order, with the id of each one's newest message. */
export interface DmSource<T> {
  readonly dms: readonly T[];
  readonly activity: (dm: T) => string | undefined;
}

/**
 * Every deployment's DMs in one list, the most recently active first. Activity is the newest
 * message's id, a UUIDv7 ordered by time, so ids from different deployments compare; DMs with
 * no messages yet follow, in each deployment's own order, the earlier sources first.
 */
export function mergeDms<T>(sources: readonly DmSource<T>[]): T[] {
  const all = sources.flatMap((source, from) =>
    source.dms.map((dm, index) => ({ dm, from, index, active: source.activity(dm) })),
  );
  all.sort((a, b) => {
    if (a.active !== undefined && b.active !== undefined && a.active !== b.active) {
      return b.active.localeCompare(a.active);
    }
    if ((a.active === undefined) !== (b.active === undefined)) {
      return a.active === undefined ? 1 : -1;
    }
    return a.from - b.from || a.index - b.index;
  });
  return all.map((entry) => entry.dm);
}
