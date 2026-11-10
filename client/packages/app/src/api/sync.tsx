import { AspenSync, type AspenClient } from "@aspen/protocol";
import { useEffect, useMemo, type ReactNode } from "react";
import { useMessages } from "@/i18n/context";
import { reportActivity } from "./activity";
import { DeploymentsProvider } from "./deployments";
import { guardSending, wordSendingGuard } from "./sendingGuard";
import { tellUploadFailures, wordUploadFailures } from "./uploadFailures";
import { nativeSendsUploads, nativeUploadTransport, wordNativeUploads } from "./uploadsBridge";
import { AspenSyncContext, HomeSyncContext } from "./syncContext";

/**
 * Owns the `AspenSync` for the signed-in session at home: it bootstraps on mount and stops,
 * forgetting the cache, on unmount, and meanwhile reports the user's activity so they show as
 * away when they stop using the app. The other deployments the user signs in to from home are
 * owned beneath it (`DeploymentsProvider`). Mount it only while a session exists, so signing out
 * unmounts it and signing in mounts a fresh one.
 */
export function SyncProvider({ client, children }: { client: AspenClient; children: ReactNode }) {
  const sync = useMemo(
    () =>
      new AspenSync({
        client,
        ...(nativeSendsUploads() ? { uploadTransport: nativeUploadTransport(client) } : {}),
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
    const stopGuarding = guardSending(sync);
    const stopTelling = tellUploadFailures(sync);
    return () => {
      stopTelling();
      stopGuarding();
      stopReporting();
      sync.stop();
    };
  }, [sync]);
  const m = useMessages();
  useEffect(() => {
    wordSendingGuard(m);
    wordUploadFailures(m);
    wordNativeUploads(m);
  }, [m]);
  return (
    <HomeSyncContext.Provider value={sync}>
      <AspenSyncContext.Provider value={sync}>
        <DeploymentsProvider homeSync={sync}>{children}</DeploymentsProvider>
      </AspenSyncContext.Provider>
    </HomeSyncContext.Provider>
  );
}
