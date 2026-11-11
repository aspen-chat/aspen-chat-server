import { ApiProblemError } from "@aspen/protocol";
import { useCallback, useEffect, useState } from "react";

/** One page of a log as the server answers it. */
export interface LogPage<T> {
  data: T[];
  /** Asks for the page after this one; `null` when this one is the last. */
  nextCursor: string | null;
}

/** A log read a page at a time by cursor, as one of the dashboard's tables shows it. */
export interface CursorLog<T> {
  /** The entries read so far, in the server's order. */
  entries: readonly T[];
  loading: boolean;
  error: string | null;
  /**
   * Whether a page follows those read: the last answer carried a cursor. Never judged by how
   * many a page held, since a page may be short, or full, and either may be the last.
   */
  more: boolean;
  /** Reads the page after those read, or the same page again after it failed. */
  showMore: () => void;
  /** Reads the log again from its first page. */
  reload: () => void;
}

/**
 * Reads a log a page at a time: the first page when mounted and whenever `load` changes, and
 * the next on `showMore`, asked for by the cursor the page before answered with. `load` must
 * keep its identity between renders (`useCallback`) for as long as it reads the same log.
 */
export function useCursorLog<T>(load: (cursor?: string) => Promise<LogPage<T>>): CursorLog<T> {
  const [entries, setEntries] = useState<readonly T[]>([]);
  // The cursor of the page after those read; `null` when the server said there is none.
  const [next, setNext] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Another `load` reads another log, from its first page.
  const [loaded, setLoaded] = useState(() => load);
  if (loaded !== load) {
    setLoaded(() => load);
    setLoading(true);
  }

  const read = useCallback(
    (cursor: string | undefined, isCurrent: () => boolean) => {
      load(cursor).then(
        (page) => {
          if (!isCurrent()) {
            return;
          }
          setEntries((held) => (cursor === undefined ? page.data : [...held, ...page.data]));
          setNext(page.nextCursor);
          setError(null);
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
    read(next, () => true);
  }, [next, read]);

  const reload = useCallback(() => {
    setLoading(true);
    read(undefined, () => true);
  }, [read]);

  return { entries, loading, error, more: next !== null, showMore, reload };
}
