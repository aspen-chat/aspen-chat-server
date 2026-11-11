import { ApiProblemError, type UserPage } from "@aspen/protocol";
import { useCallback, useEffect, useState } from "react";

/** A list read a page at a time by cursor, as a dialog shows it. */
export interface CursorPages {
  /** The ids read so far, in the server's order; the records themselves are in the store. */
  ids: readonly string[];
  loading: boolean;
  error: string | null;
  /**
   * Whether a page follows those read: the last answer carried a cursor. Never judged by how
   * many a page held, since a page may be short, or full, and either may be the last.
   */
  more: boolean;
  /** Reads the page after those read, or the same page again after it failed. */
  showMore: () => void;
  /** Takes one id out of those shown, when what it stood for is removed here. */
  drop: (id: string) => void;
}

/**
 * Reads a list of people a page at a time: the first page when mounted and whenever `load`
 * changes, and the next on `showMore`, asked for by the cursor the page before answered with.
 * `load` must keep its identity between renders (`useCallback`).
 */
export function useCursorPages(load: (cursor?: string) => Promise<UserPage>): CursorPages {
  const [ids, setIds] = useState<readonly string[]>([]);
  // The cursor of the page after those read; `null` when the server said there is none.
  const [next, setNext] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(
    (cursor: string | undefined, isCurrent: () => boolean) => {
      load(cursor).then(
        (page) => {
          if (!isCurrent()) {
            return;
          }
          const found = page.data.map((u) => u.id);
          // Someone listed again further on (their reaction taken back and made again between
          // two pages) keeps the one place they already have.
          setIds((held) =>
            cursor === undefined ? found : [...held, ...found.filter((id) => !held.includes(id))],
          );
          setNext(page.nextCursor);
          setLoading(false);
        },
        (e: unknown) => {
          if (isCurrent()) {
            setError(e instanceof ApiProblemError ? e.message : String(e));
            setLoading(false);
          }
        },
      );
    },
    [load],
  );

  useEffect(() => {
    let current = true;
    read(undefined, () => current);
    return () => {
      current = false;
    };
  }, [read]);

  const showMore = useCallback(() => {
    if (next === null) {
      return;
    }
    setLoading(true);
    setError(null);
    read(next, () => true);
  }, [next, read]);

  const drop = useCallback((id: string) => {
    setIds((held) => held.filter((other) => other !== id));
  }, []);

  return { ids, loading, error, more: next !== null, showMore, drop };
}
