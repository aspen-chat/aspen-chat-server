import { useMemo, useSyncExternalStore, type ReactNode } from "react";
import { AccountsContext, AccountsHub } from "./accounts";
import { AspenClientContext, HomeClientContext } from "./context";

/**
 * Owns this install's accounts on the chosen server (`AccountsHub`). A new server address means
 * a new hub. Until an account's own scope says otherwise, the client in reach is the front one's.
 */
export function AspenProvider({ serverUrl, children }: { serverUrl: string; children: ReactNode }) {
  const hub = useMemo(() => new AccountsHub(serverUrl), [serverUrl]);
  const view = useSyncExternalStore(hub.subscribe, hub.view);
  const client = (view.entries.find((e) => e.account.id === view.front) ?? view.entries[0])?.client;
  if (client === undefined) {
    throw new Error("an install always holds an account");
  }
  return (
    <AccountsContext.Provider value={hub}>
      <HomeClientContext.Provider value={client}>
        <AspenClientContext.Provider value={client}>{children}</AspenClientContext.Provider>
      </HomeClientContext.Provider>
    </AccountsContext.Provider>
  );
}
