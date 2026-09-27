import { AspenSync, type AspenClient } from "@aspen/protocol";
import { useEffect, useMemo, type ReactNode } from "react";
import { reportActivity } from "./activity";
import { AspenSyncContext } from "./syncContext";

/**
 * Owns the `AspenSync` for the signed-in session: it bootstraps on mount and stops, forgetting
 * the cache, on unmount, and meanwhile reports the user's activity so they show as away when
 * they stop using the app. Mount it only while a session exists, so signing out unmounts it and
 * signing in mounts a fresh one.
 */
export function SyncProvider({ client, children }: { client: AspenClient; children: ReactNode }) {
  const sync = useMemo(
    () =>
      new AspenSync({
        client,
        validateEvents: import.meta.env.DEV,
        onInvalidEvent: (raw, errors) => {
          console.warn("dropped a server event that does not match the schema", errors, raw);
        },
      }),
    [client],
  );
  useEffect(() => {
    sync.start();
    const stopReporting = reportActivity(sync);
    return () => {
      stopReporting();
      sync.stop();
    };
  }, [sync]);
  return <AspenSyncContext.Provider value={sync}>{children}</AspenSyncContext.Provider>;
}
