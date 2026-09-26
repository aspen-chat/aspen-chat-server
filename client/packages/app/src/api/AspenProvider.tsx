import { AspenClient, WebStorageSessionStore } from "@aspen/protocol";
import { useMemo, type ReactNode } from "react";
import { AspenClientContext } from "./context";

/** Owns the one `AspenClient` for the chosen server. A new server address means a new client. */
export function AspenProvider({ serverUrl, children }: { serverUrl: string; children: ReactNode }) {
  const client = useMemo(
    () =>
      new AspenClient({
        baseUrl: serverUrl,
        sessionStore: new WebStorageSessionStore(window.localStorage),
      }),
    [serverUrl],
  );
  return <AspenClientContext.Provider value={client}>{children}</AspenClientContext.Provider>;
}
