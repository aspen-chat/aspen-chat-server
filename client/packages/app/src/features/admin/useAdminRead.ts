import { ApiProblemError } from "@aspen/protocol";
import { useCallback, useEffect, useState } from "react";

/** The state of one of the dashboard's reads. */
export interface AdminRead<T> {
  /** The last answer, kept while the next is on its way. */
  data: T | undefined;
  /** Why the last read failed, as the server put it; `null` once one succeeds. */
  error: string | null;
  /** When the last answer arrived, in milliseconds since the epoch; for "how long ago". */
  at: number;
  reload: () => void;
}

/**
 * Runs one of the dashboard's reads, again whenever `load` changes and, with `every`, every that
 * many milliseconds while the page is visible. The dashboard's figures are the server's answer
 * of the moment rather than cached records, so they are held here, where they are shown.
 */
export function useAdminRead<T>(load: () => Promise<T>, every?: number): AdminRead<T> {
  const [data, setData] = useState<{ answer: T; at: number } | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    let current = true;
    load().then(
      (answer) => {
        if (current) {
          setData({ answer, at: Date.now() });
          setError(null);
        }
      },
      (e: unknown) => {
        if (current) {
          setError(e instanceof ApiProblemError ? e.message : String(e));
        }
      },
    );
    return () => {
      current = false;
    };
  }, [load, generation]);

  useEffect(() => {
    if (every === undefined) {
      return;
    }
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") {
        setGeneration((n) => n + 1);
      }
    }, every);
    return () => {
      clearInterval(timer);
    };
  }, [every]);

  const reload = useCallback(() => {
    setGeneration((n) => n + 1);
  }, []);
  return { data: data?.answer, at: data?.at ?? 0, error, reload };
}
