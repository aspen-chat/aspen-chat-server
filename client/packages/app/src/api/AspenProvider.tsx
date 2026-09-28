import { AspenClient, WebStorageSessionStore } from "@aspen/protocol";
import { useMemo, type ReactNode } from "react";
import { AspenClientContext, HomeClientContext } from "./context";

/**
 * Owns the `AspenClient` for the chosen server, the user's home. A new server address means a
 * new client.
 */
export function AspenProvider({ serverUrl, children }: { serverUrl: string; children: ReactNode }) {
  const client = useMemo(
    () =>
      new AspenClient({
        baseUrl: serverUrl,
        sessionStore: new WebStorageSessionStore(window.localStorage),
      }),
    [serverUrl],
  );
  return (
    <HomeClientContext.Provider value={client}>
      <AspenClientContext.Provider value={client}>{children}</AspenClientContext.Provider>
    </HomeClientContext.Provider>
  );
}
