import { AspenSync, type AspenClient } from "@aspen/protocol";
import { useEffect, useMemo, type ReactNode } from "react";
import { useMessages } from "@/i18n/context";
import { reportActivity } from "./activity";
import { DeploymentsProvider } from "./deployments";
import { guardSending, wordSendingGuard } from "./sendingGuard";
import { tellUploadFailures, wordUploadFailures } from "./uploadFailures";
import { wordCallNames } from "./callName";
import { nativeCallFactory } from "./nativeCallFactory";
import { nativeSendsUploads, nativeUploadTransport, wordNativeUploads } from "./uploadsBridge";
import { AspenSyncContext, HomeSyncContext } from "./syncContext";

/**
 * Owns the `AspenSync` for an account's signed-in session at home: it bootstraps on mount and
 * stops, forgetting the cache, on unmount. While the account is `attended`, in front of the
 * person, it reports their activity, so they show as away when they stop using it; an account
 * behind it stays connected and shows as away. The other deployments the account signs in to from
 * home are owned beneath it (`DeploymentsProvider`). Mount it only while a session exists, so
 * signing out unmounts it and signing in mounts a fresh one.
 */
export function SyncProvider({
  client,
  attended,
  children,
}: {
  client: AspenClient;
  attended: boolean;
  children: ReactNode;
}) {
  const sync = useMemo(() => {
    // The call's system name reads the sync's store, which exists only once the sync is made.
    const made: { sync: AspenSync | null } = { sync: null };
    const sync = new AspenSync({
      client,
      ...(nativeSendsUploads() ? { uploadTransport: nativeUploadTransport(client) } : {}),
      ...nativeCallFactory(() => made.sync),
      validateEvents: import.meta.env.DEV,
      onInvalidEvent: (raw, errors) => {
        console.warn("dropped a server event that does not match the schema", errors, raw);
      },
    });
    made.sync = sync;
    return sync;
  }, [client]);
  useEffect(() => {
    sync.start();
    const stopGuarding = guardSending(sync);
    const stopTelling = tellUploadFailures(sync);
    return () => {
      stopTelling();
      stopGuarding();
      sync.stop();
    };
  }, [sync]);
  useEffect(() => (attended ? reportActivity(sync) : undefined), [sync, attended]);
  const m = useMessages();
  useEffect(() => {
    wordSendingGuard(m);
    wordUploadFailures(m);
    wordNativeUploads(m);
    wordCallNames(m);
  }, [m]);
  return (
    <HomeSyncContext.Provider value={sync}>
      <AspenSyncContext.Provider value={sync}>
        <DeploymentsProvider homeSync={sync} attended={attended}>
          {children}
        </DeploymentsProvider>
      </AspenSyncContext.Provider>
    </HomeSyncContext.Provider>
  );
}
